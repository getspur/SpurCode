# M04 worker evidence — bd-6doq

Source-only migration `898959ebdbba5e414ef35b51e7ed2543f7103116` replaces eight named None fields with four `..GraphIndexArtifact::empty()` tails in the three approved fixture files. Independent byte reconstruction verified every other source byte, all original assertions and the Arc wrapper. All temporary observers were removed before final validation. ROOT POST, independent review and approval remain pending; the canonical issue stays in_progress for review.

Original `809974cabc121537a4ce243931808f8555fad631`; unchanged-test baseline `5e82035ddc8b4e67cc8a6118eeb25a758c3c436e`; authenticated native baseline checkpoint `da03d07f0f4193ed1725fa4b7c56b9fd8b9a34a7`; native equality checkpoint `1d656f9aeb2f4d5881837c7fc9716b81466cab75`. The final returned packet commit is retained at `refs/evidence/M04-f077cfd7/final`. All earlier uniquely named refs remain available. Complete IDs are in `commits.json`.

| Actual source | Actual constructor | Actual hash input | Before / after records | Native value join |
| --- | --- | --- | --- | --- |
| `overlay_runtime.rs` | `tests::empty_artifact` | `base` | 1 / 1 | equal |
| `rebuild_singleflight.rs` | `tests::artifact` | `first` | 1 / 1 | equal |
| `request_cache.rs` | `tests::empty_artifact` | `one` | 1 / 1 | equal |
| `request_cache.rs` | `tests::write_empty_parquet` | `cache-test-hash` | 1 / 1 | equal |

The four named owning cases and real input identities are in `equality-join.json`. Whole wire/Debug strings, skipped node IDs, eleven vector lengths and capacities, typed None states and native canonical/capability hashes match exactly. Both same-named empty helpers are file-qualified. The Parquet fixture was exported before its original write call. Raw numeric thread IDs, sequence/caller positions and random destination directories remain in `before-native.json.gz`, `after-native.json.gz` and raw logs. Equality joins actual named case/file/function/hash input and multiplicity; diagnostic thread IDs/sequence, caller line shifts and destination paths are not artifact values. Fifteen independently mutated copies of actual records reject: ten exported value fields, file/site/named case, hash input and multiplicity. Python authenticates/joins exports; Rust computes native hashes and capability identities.

| Original command through scripts/spur-cargo | Baseline | Final |
| --- | --- | --- |
| test -p spur-graph --lib, unfiltered | 589 pass / 0 fail / 3 ignore / 0 filter | 589 pass / 0 fail / 3 ignore / 0 filter |
| test -p spur-graph --test generic_evidence_providers, unfiltered | 16 pass / 0 fail / 0 ignore / 0 filter | 16 pass / 0 fail / 0 ignore / 0 filter |
| Four exact owning cases, observation only | 4 pass / 0 fail / 0 ignore / 588 filter | 4 pass / 0 fail / 0 ignore / 588 filter |
| check -p spur-graph --all-targets | final-only required check | exit 0 |
| fmt -p spur-graph -- --check, local | final-only scoped format check | exit 0 |
| source/evidence git diff --check from PRE base | final-only required check | exit 0 |

The original and final complete named library/provider test status sets are equal. The three original model-download ignores are: `store::lance_sections::tests::coderank_embed_smoke_constructs_model_and_returns_expected_dimensions`, `store::lance_sections::tests::fastembed_smoke_constructs_model_and_returns_expected_dimensions`, `store::lance_sections::tests::jina_code_smoke_constructs_model_and_returns_expected_dimensions`. No new ignores, filters on required unfiltered suites, assertion changes, limits, timeout tuning or substituted tests were introduced. No behavioral RED was fabricated.

Every remote build/test/check used SPUR_REMOTE=1, SPUR_NO_LOCAL_FALLBACK=1, SPUR_SYNC_FULL=1 and SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0 through scripts/spur-cargo. Every command archived argv/environment/UTC timestamps/exit, all 133 source hashes and stable HEAD before/after. `*.command.json` and `*.log.gz` retain the complete command streams, counts and names. Native readbacks authenticate the exact saved observed sources and record executable SHA256/size, rustc 1.94.1 verbose identity, aarch64 host and manifest directory. Exact patches are losslessly compressed; full original/observed/unobserved bytes are saved as source JSON gzip archives. `verify.py`, `verification-result.json`, `final-source-bindings.json` and `SHA256SUMS.json` bind the final packet.

PRE replay passed 133 source inputs, 4 sites, 9 prospective cases, 2 lanes, 33 variables, 8 independent predicates and 10 false controls at fingerprint e8d11f990f449e8c667d16e45e980f4f52b67aee2830a5b51dea5511e5176439. Complete live sol_7115d0e7e60f4f17 SAT/pass and sol_e0e1bcf2b5d24b1a UNSAT/fail parsed exactly equal the archived reloaded envelopes; full envelopes are in pre-live.json. PRE is prospective; the actual evidence above is separate. Existing M03 pins were untouched, and retired M02 receipts remain archived. INTENT and ACTION audits persisted with identical idempotent replays (comments 4776 and 4782); the RECORD request/receipts are archived separately.

Preserved diagnostics: the first sandbox remote attempt exited 200 for AWS connectivity and forbade local fallback; the authorized network-enabled retry passed. The initial Git metadata checkpoint hit sandbox index.lock denial before succeeding with authorized access. Six graph symbol reads exceeded the tool deadline, so declared source bytes were used. The runtime duration risk signal is archived, with a later same-assignment lease in final-runtime.json; ROOT owns its label reconciliation. The first mutation report accidentally aliased the original capacity/length labels; its exact export is preserved in first-control-export-metadata-alias.json.gz and correction details in control-export-correction.json. Corrected labels authenticate against the actual records; native equality and all 15 rejection outcomes were unaffected. Remote sccache aggregate counters are diagnostic and are not test failures or performance claims.

Inherited bd-3dkq, bd-1izf, bd-3umt and bd-16eq remain open, with full readbacks in inherited-issues.json. No shared cause or repair is claimed. No new product semantic difference was observed in this scoped batch. No main merge, closure, self-approval, successor dispatch or ROOT POST was performed. `root-post-candidates.json` supplies actual source/native-bound review candidates; ROOT supplies fresh Jev POST and independent review before integration/M05.

Replay the evidence locally with `python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M04/worker/verify.py`; frozen PRE replays without --current-source. Native logs and their exact commands remain the evidence for remote execution.
