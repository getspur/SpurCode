# C attempt 2: rework required

Canonical issue `bd-1ktd`, delegation `c857c0f4b700428b`. Root reviewed authored `c70cc048fb9940b6c0262a1c551d19ac476491a3`; normalized wrapper `716fc4cdab430765504c69d82472f8c95b6e65c8` has the same tree. Orchestrator completion audit 3178 exists despite the worker evidence endpoint's authorization failure. C remains incomplete and packaging stays blocked.

## Results that are supported

The independent `audit_c2_vectors.py` checks raw captured vectors against the pinned S4C reference bytes, finite dimensions/norms, model identities, request/generation correlation and native child wait records. All three models have two analyst-owned lifecycles; maximum query difference is below `3.707e-9`. Separate graph-owned batches cover both distinct reference inputs, with maximum difference `4.142e-9`. The saved CLI indexed-pack response is answerable, high confidence and hybrid-grounded, and its CLI/helpers exited zero. These are reviewed C2 captures, not newly executed root model benchmarks. The v1 test calls the handler directly and repeats the query; socket transport is separately tested. A graph batch owner is distinct from the analyst owner.

The worker's own verifiers and corruptions also replayed successfully, and its POST candidate file reproduced byte-for-byte. Those results do not erase the failures below.

## Blocking review findings

1. Root's fresh analyst run found `embedding_sidecar_layout::sidecar_modules_stay_within_refactor_budgets` failing. The library portion passed 107 tests with three explicit real-helper ignores; embedding then reported 28 passed and one failed, so later targets did not run. The newly appended real acceptance module makes `sidecar_service.rs` 315 lines, violating the existing strict limit of 300. Move the test body to a separate module/file; preserve the cap and test.
2. The standalone public `query_context_candidates` path still opens a connection that loads ICU but not FTS when autoload is disabled. C2 changed its confidence test to an internally preloaded connection, so that pass did not fix or test the public route. Root test commit `d3b708e318328c25fcca3c8cad1053df5a0a7363` restores public-API coverage before any production fix. `run_public_query_red.py` independently ran the real pinned-helper fixture and recorded exit 101, failing on missing FTS `stem`. The descriptor/model fixture children exited zero; this failure is not a native abort or one of the eight graph baselines.
3. Root's feature-enabled client suite passed 39 tests but again printed `kill: illegal process id:`. `owner_can_join_cancelled_descriptor_process` cancels when a PID file merely exists and can read it before content is written. A failed invalid-argument probe is not child-absence evidence. Synchronize on a parsed positive PID and require a meaningful cleanup/absence probe; include deterministic empty-file/invalid-input coverage. This is a test-evidence repair, not a new native ownership redesign.

No production source was changed by root. The restored public-query test is intentionally RED. Root's own initial vector auditor assumed the service had no startup arguments; inspection showed numeric limit arguments, and the classifier was corrected to distinguish descriptor invocations by `--describe-model`. The underlying captured vectors/events were unchanged.

## Executed POST and supplemental PRE

All four fresh Jev interpretations opened their gates. Catalog tuple/uniqueness/cardinality checks accepted the two C model/lifecycle snapshots and rejected their twelve corruptions. Separate actual-observation regression checks rejected both the over-budget source and the public-query/PID defects. C therefore is not approved.

Supplemental C3 PRE is executed: `sol_4195bee1755e4ddc` verifies the feasible test-module split against the existing cap; `sol_1e97e3f7cebc436d` verifies the public-query/PID repair contract. Five adverse variants reject. Strict `<300` translates to inclusive `299`; the derived split projection is 290 lines, a feasible example rather than a required final count or a new limit. POST must use the actual final source count.

`solver-records.json.gz` contains all 23 exact requests, solver outputs and pinned/reloaded lowerings. `Jev.json.gz` preserves intent/gates/provenance; `catalog.json.gz` preserves rule contracts; `receipts.json` lists IDs. `verify_review.py` independently checks 63 lowered hard predicates and the runtime failures. Its successful exit verifies this rejection-and-PRE packet; it does not say C is GREEN.

The solver cache reached its 512-receipt limit. Root rechecked and released only twenty approved B cache pins whose full receipts were already committed at `4a02c151d50e9b32f12caac09d402717619ed779`. Their archives remain intact; original C PRE and fresh C receipts stay pinned. Exact released IDs are archived here.

## Handoff

Use the protected review history, including the committed public-API failing test and this packet, before implementing C3. Fix the three findings through test/RED/fix/GREEN; retain all prior real evidence with accurate source/build labels. Narrowly replay affected real inference after moving the test or fixing the query path; unchanged model captures can remain historical C2 evidence with an explicit final-source equivalence check. Do not reattribute old runs to new binaries.

Preserve Off/Sidecar/Auto, v1 compatibility, cancellation/error/no-late-publication, checked shutdown and native-free product dependencies. Windows, the eight documented direct-native graph baselines, packaging and the notebook experiment remain outside the passing observations. No whole-project panic-detection or native-memory-safety claim is made.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-03-embedding-isolation/S6/replan/root-C2-review/verify_review.py
```
