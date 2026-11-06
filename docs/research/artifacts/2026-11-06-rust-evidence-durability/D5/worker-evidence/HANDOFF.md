Persist the canonical authenticated envelope unchanged so full, slim and private delta snapshots retain Rust evidence and reject incomplete or altered transport.

D5 is implemented in code commit `26136e0ccd05eaadb37458a2f233a6ba3698909b`; this packet is a worker candidate for ROOT review and fresh actual-output POST/Jev. The required analyst command is **not GREEN**: its unchanged integration failure also reproduces against the exact dispatch-base production. ROOT disposition is required. Canonical issue `bd-73kv` remains `in_progress`, assigned to `spur-worker-6e0cb01540424715`; the branch UUID was never used as the delegation identity.

[Unified implementation diff](implementation.diff) (zero context; [standard context diff preserved losslessly](implementation.diff.gz)) covers exactly nine authorized crate files. The graph extractor and resolver constants, collector algorithms, dependencies, cache production, query/MCP/CLI/runtime code and external repositories are unchanged. The only analyst edit is the optional manifest fixture member. The five transitional current-seed tests now verify incremental reuse, exact unchanged payloads, saved source/provenance, selected new-source identity and exact stats. The edited carrier is compared with the selected native capture; its complete origin inventory is preserved separately from the fresh-full graph comparison.

| Commit | Purpose |
| --- | --- |
| `92c22be246986df954536447fddf833488a47aa9` | Existing-API zero-source regression before runtime RED |
| `9f6aa0b6edd9b9aa516313b07f522d6a534721af` | Strict transport and corruption regressions before implementation |
| `caf7436ea8861541ef4c09e6e7f1a9a0bdbe6cea` | Current-seed, retained-origin and valid-cell checksum tests |
| `26136e0ccd05eaadb37458a2f233a6ba3698909b` | Complete D5 transport implementation |

PRE verification ran at dispatch base `c511ff02451bbddaba527057b4dca620104893e7`: 75 bindings, three receipts, two SAT/one UNSAT and 22 independent predicates. All three fresh `get_solve_result` envelopes exactly match `root-pre/proof.json.gz`, including complete lowered requests, model/core and status; the lossless live envelopes are [preserved](pre-live.json.gz). INTENT audit `33270b15-6b28-455b-bbb7-8b32ced38485` is comment 4055 and replayed idempotently. The RED checkpoint audit `4b6f61c5-0a16-488e-8b8b-cc768c1b4556` is comment 4058 and replayed identically.

The first regression compiled on the authorized remote builder and failed at the actual assertion `out.path().join("rust_evidence_sources.parquet").is_file()`: exit 101, zero passed/one failed. Its complete raw log SHA-256 is `002257d5124a9de948d1a19c4ed15c5bd8083f463004aaef2699c1e3a0783849`; executable SHA-256 is `db154f6a01d258e57130fe30afccfbc29cd07b88475f3b2e061e8f5ee827c7e6`. This is the production assertion RED. A subsequent `MethodCallSourceScope` move error was corrected with `.clone()`; that compiler failure is preserved separately.

All build/check/test commands used `scripts/spur-cargo` with `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0`. Formatting was local. Exact argv, environment, exit codes, timestamps, source bindings and raw lossless logs for every attempt are in [run-results.json](run-results.json) and [runs](runs/). No remote RED was rerun locally.

| Final verification | Exit | Observed result |
| --- | --- | --- |
| `final-suites`: all seven scoped graph integration targets | 0 | 71 passed; one pre-existing cross-device ignore |
| `final-transport`: Rust evidence Parquet | 0 | 16 passed |
| `final-check`: graph `--all-targets` | 0 | All targets compile; existing warnings remain |
| `final-fmt`: graph and analyst formatting | 0 | Clean |
| `analyst-overlay`: required broad filter | 101 | 18 unit + five integration tests passed; one integration failure |
| `analyst-baseline`: unchanged failing test on exact base production | 101 | Same line 158, zero edges versus one expected |
| `final-analyst-control-2`: same test on committed D5 | 101 | Same failure |

The final scoped batch is exactly:

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0 scripts/spur-cargo test -p spur-graph --test rust_evidence_parquet --test macro_invocation_parquet --test parquet_roundtrip --test parquet_schema_invariants --test load_artifact_dispatch --test diagnostics_persist --test integration_git_incremental --no-fail-fast --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D5/worker-evidence/native-export-env.toml -- --test-threads=1
```

The analyst failure is [recorded separately](analyst-baseline-risk.json). A reversible baseline projection restored all affected production bytes to the dispatch commit; only uncompiled D5 graph test files differed. The scoped files were restored byte for byte afterward, and final current-source checks were rerun. The baseline executable SHA-256 is `01358ecdcdd85106b39af8ee50ee313704cf19f03664d51bb2244e2cf68c5545`; the final current-source failing executable SHA-256 is `0ec94a975c21a5fda528c44d92eb2cfea2f38d2238d7cf535fe810d5d537b7a3`. No analyst overlay or incremental production fix was attempted. Risk signal `e1144892-0549-4f53-980d-2c4a3df97770` is durable.

The [final native archive](native-outputs.tar.gz) contains whole manifests and Parquet bytes plus actual schemas, all metadata and rows, original and reconstructed full/slim envelopes, source bytes as lossless hex, native capability digests and native validator verdicts. It contains baseline/saved snapshot, complete-zero source, Some-empty, None, Unknown lanes, composed two-origin snapshots, changed/zero/Unknown/no-change/deletion deltas, explicit legacy-v12 absence and all saved corruption controls. Rehashed existing-body and owner forgeries pass transport integrity and fail validation against saved source bytes. History-only remains None in the executed regression; no slim/history canonical artifact hash is claimed.

[Independent expectations](independent-expected.json) derive source OIDs/lengths and call/body geometry from preserved raw bytes and pre-transport marker ranges. [The observer](verify-native.py) verified 2,643 captured files and 155 controls rejected by both readers, including all five same-count cell controls. It checks capability equality and retained complete origin inventories. It does not replace ROOT's source authentication, solver or review. [The native replayer](replay-native.py) copies whole snapshots and invokes a supplied current native test executable for fresh zero-row source deletion, same-count call mutation and checksum/schema controls; its usage is in its docstring. Run it on a host compatible with that executable. ROOT verifies a fresh executable/source identity before running it.

Final source binding SHA-256: `8fb6a3451bb724814874722f02ad5dbb06576d719a22785a113d5ebd06e6500a`. All 1,872 local bindings match the remote capture; the capture-only environment config is pinned separately. [Native identities](native-identities.json), [measurement manifest](measurement-manifest.json) and [source match](source-match.json) preserve exact compiler, executable and source identities, distinguishing the historical RED binary and the analyst failure from final passing binaries. Native compiler: Rust 1.98.1, `aarch64-unknown-linux-gnu`, LLVM 22.1.8. Final transport executable SHA-256: `e10ac45dcb5e4a14017782992530d9af74fd2b2d682be60feafed17a7474db09`.

Final native archive SHA-256: `5d4594aabec0b2f200f20e7d3f1e8a4c54adf2895d83a192d5fa5473655157df`. The earlier `intermediate-native-outputs.tar.gz` and `transport16-native-outputs.tar.gz` preserve preceding captures without replacing them with expected summaries. Original historical packets and solver storage/pins are unchanged.

Infrastructure attempts are classified separately: restricted-network EC2 failure exit 200; exhausted rsync retries exit 30; a revision-guard exit 1 before tests when the code commit overlapped the last diagnostic upload. The stable committed-source retry completed and reproduced the analyst failure. The initial native export lookup used workspace cwd while Cargo tests had used crate cwd; the actual initial bytes were recovered and preserved, and the adapter now resolves exports from the workspace. No infrastructure, quota, native extension or dependency was modified, and no cold local build, push, publication, merge, closure, self-approval or D6 work occurred.

Completion audit persistence is **unconfirmed**. The final request UUID is `26a0ad6f-8b57-49d4-b55e-ae2d489ad64c`; both its initial submission and identical replay received no MCP response within the local 55-second deadline. An extra replay of the already-confirmed RED checkpoint returned the server error `timed out awaiting tools/call after 300s`. The exact requests and outcomes are preserved in [completion-record.json](completion-record.json) and [red-audit.json](red-audit.json). ROOT can retry [completion-request.json](completion-request.json) unchanged; the request may still complete. The final `report_progress` acknowledgment was also unconfirmed. ROOT still owns independent verification, the existing analyst failure disposition, fresh actual-output POST/Jev and approval.
