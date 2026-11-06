# ROOT D5 attempt-1 review — changes required

D5 is not approved or merged. Local main remains f51d56931. The reviewed normalized worker tip is c49409a14; the original test/fix/evidence history is protected by spur/review-evidence/g3-d5-attempt1-authored at f2a4e6ea4. Exactly nine authorized crate files changed; their bytes match the original authored history. ROOT added only this review evidence.

Durable Beads completion audit 4065 is confirmed, along with INTENT 4055, RED 4058, and the risk records. The worker's own final evidence audit still has no confirmed response; ROOT has not claimed it was persisted. Read-only comments were archived in beads-audits.json. All 1,872 local source bindings match the final worker measurements, and all 2,643 files in the SHA-bound native archive were independently verified.

Two repairs are required:

1. The Rust absent-evidence read branch parses the schema suffix as a number and accepts v012 and v+12 as legacy v12. Fresh native ROOT probes verified 1,883 remote source hashes and binary SHA e10ac45dcb5e4a14017782992530d9af74fd2b2d682be60feafed17a7474db09 before execution. Both readers loaded both undeclared aliases. The canonical v12 absence control loaded; v999 rejected with the rebuild diagnostic. native-version-probes.tar.gz contains the complete copied disk snapshots, actual manifests, native output JSON/logs and source identities. Its SHA-256 is d37a0f8efb912fa5bfc227728064bfeea777fdf8d914265ee45e2e65a8821af4. Macro evidence was explicitly removed in these controls to exercise Rust absence version recognition independently.
2. The complete unchanged extractor target returned exit 101: 53 passed, two failed, three existing ignores. The failures are graph_store_schema_version_is_v12 and rust_extractor_matches_sample_corpus_golden_artifact. The guard retains the old v12 expectation; the Rust golden retains the old origin manifest version. All five non-Rust goldens passed. The raw lossless log is extractor-runtime-RED.log.gz. The preceding sandbox-network attempt exited 200 before tests; it is preserved separately and is not a test RED.

The ROOT extractor command was:
```bash
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0 \
SPUR_CLOUD_BUILD_SH=/Users/kevintruong/project/SpurCode/scripts/cloud-build/build.sh \
scripts/spur-cargo test -p spur-graph --test extractor \
  --target-dir /mnt/cargo/targets/spur/worktrees/00c43777-38c2-4eef-b6c4-36a33474d37b -- --nocapture
```

The analyst overlay failure remains explicit and is tracked separately in bd-16eq, after reproducing on exact pre-D5 production with unchanged test and overlay code. This is a scoped disposition, not a GREEN claim or waiver of overall durability acceptance. No analyst overlay production repair belongs in this retry.

The [rework contract](FEEDBACK.md) authorizes only the strict legacy identity guard, direct extractor version/golden test fallout and their regressions. The original source-grounded PRE and committed runtime TDD history remain preserved. The supplemental [PRE verifier](rework-pre/verify.py) binds 1,902 current source/evidence files and independently evaluates 15 finite/lowered predicates. Fingerprint: 9fce14f4f133d2b2e2ed91bc4d30711ca9d3ed17cf32df988ab3d8128a4e80d4.

| Receipt | Meaning | Expected |
|---|---|---|
| sol_10a048ed91254fbb | Prospective exact schema and v13 test expectations | SAT |
| sol_2eac73069be1464a | Actual native alias acceptance and stale test expectations | UNSAT |

Both complete results were persisted, pinned and reloaded. The invalid model has three independently failing exact-tuple bindings. Its observations remain intentionally failing; this PRE does not claim a fix. Jev's first transport request failed before any solver or pin mutation. The identical retry returned an open verify request for versions_exact with identical facts; all other required rules were explicitly added from the current catalog.

Retention: before releasing two superseded closed-D3 POST pins, ROOT archived and exactly compared complete live envelopes and independently evaluated their six predicates. The records are under rework-pre/retired-archives.json.gz, retired-live.json.gz and retirement-operations.json. Only pin=false was used. All seven current D4 POST pins and all three original D5 PRE pins remain protected; no solver store or quota was edited.

After clean authorized recovery of this checkpoint, the retry must run the rework verifier and fully reload its two receipts before test/production edits. It must commit and execute assertion REDs, apply minimal fixes, rerun both full extractor and original scoped suites, and return fresh native disk exports. Fresh final ROOT actual-output POST/Jev and independent approval remain required; D6 is gated.
