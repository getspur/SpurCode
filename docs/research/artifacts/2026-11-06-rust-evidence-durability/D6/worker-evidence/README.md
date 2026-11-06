# D6 worker evidence

Reject different reloaded Rust envelopes before publication, and bind base snapshots to canonical path, Rust capability and the entire declared manifest so same-source artifacts cannot alias incompatible immutable snapshots.

Canonical issue **bd-21oj**, delegation **463d0b37d3954755**, assignee **spur-worker-463d0b37d3954755**. Dispatch base `e2418efe07950cb8e0bac6569fd8da4a046670d5`. Implementation commit **0308eef43**. [Unified code/test diff](unified.diff). ROOT owns the fresh actual-output POST/Jev, independent review and integration; the issue remains `in_progress`.

Entry status, exact assignment/delegation label, INTENT request, persisted comment **4095** and identical replay are preserved in [entry.json](entry.json). The pre-edit verifier passed with fingerprint `f2a0f08094cb5137e4b7f6006b90bf7434cacb8bffb0cf486b0c1252ebd9eae9`, 1,913 bindings, 12 predicates, 38 predecessor POST predicates and six retired D3 predicates. [pre-verification.json](pre-verification.json) is its generated result. Both complete live receipts (`sol_daf5829f819643de` SAT, `sol_af1448acbb854e13` UNSAT) were reloaded through MCP and exactly compared to every field of `root-pre/proof.json.gz`; their lossless live envelopes are [pre-live.json.gz](pre-live.json.gz). No solver storage or pins were changed.

The first existing-API test was committed as **37f9882f9** before execution. It compiled and failed its required runtime assertion. Additional regressions were committed as **a704fa0fd**, **58101f0cc**, and **24147cd96** before production edits. The last commit corrects the observation helper to read CURRENT's symlink target bytes on Unix, or regular pointer-file bytes elsewhere. Capture configuration commits are **b218947aa** and **582c0fa4f**.

| Run basename | Exit | Actual result |
|---|---:|---|
| `publication-red` | 101 | Compiled; 0 passed, 1 assertion failed, 582 filtered. `validate_publication(expected, persisted_legacy)` accepted the different Rust capability. |
| `transitions-red` | 101 | Compiled; 1 passed, 7 failed, 583 filtered. Six behavioral assertion failures; the pointer probe separately failed in its helper with `IsADirectory` when reading CURRENT as a file. |
| `pointers-red` | 101 | Corrected committed probe compiled; 0 passed, 1 assertion failed, 590 filtered. All six differing-envelope direct/dedup cases published; four matching controls succeeded. |
| `cache-green` | 0 | **31 passed**, zero failures, 560 unit tests filtered. All package integration binaries were also compiled and invoked with this filter. |
| `integrations-green` | 0 | Git incremental **28 passed, 1 ignored**; Rust evidence Parquet **18 passed**. The existing cross-device test is ignored because it needs separate filesystems; the store unit test injects EXDEV. |
| `all-targets-green` | 0 | `check -p spur-graph --all-targets` passed. |
| `fmt-green` | 0 | Local `fmt --package spur-graph --check` passed. |
| `native-replay-green` | 101 | Extra probe invocation failed before compilation: the remote wrapper stripped inline TOML string quotes. This is a config failure, not an assertion RED. |
| `native-replay-file-green` | 0 | Config-file retry: **1 passed**, zero failures, 590 filtered. Three real replay packets captured. |

The replay entry returns immediately when `D6_REPLAY_REQUEST` is unset; that path contributes one pass in the cache suite and the broad RED run. The separate final replay run supplies the request and exercises the native operations.

Every basename has exact argv, environment, timestamps, exit, source/config bindings and log SHA-256 in `runs/*-command.json.gz` and `runs/*-source-bindings.json.gz`; `runs/*.log.gz` preserves the complete raw combined stream. The two earlier RED assertion logs remain intact. The argument-limit failure while fetching source identities is preserved in `runs/transitions-red-fetch-argmax.log.gz`, with its original collector in [fetch-argmax.py](fetch-argmax.py); the successful collector streams its request over stdin instead. The initial sandbox SSM probe could not reach its endpoint; approved remote access then succeeded. Git's initial sandbox index-lock denial was resolved through the authorized escalation.

All compile/test/check commands used `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0`; no local compilation retry occurred. The inactive completed D5 target was copied into the isolated D6 target after inspecting process activity; the D5 target was preserved. The VM was `i-01d560dbad5bbb79c`, region `ap-southeast-5`. Sources were fixed during each sync; logs and native outputs stayed outside the checkout until all runs finished. GREEN ran before the fix commit was created: the command records therefore name the preceding test commit, while their actual source bindings match the committed implementation exactly.

The verified implementation SHA-256 is `1b814fe9cc1d6b16c4c2a76f3a90ce78eae79ec7f7c1cedb8ab20baceceb330b`. Native captures compared every selected remote source/config file to the local pre-run bindings: 1,879 files for cache GREEN, with one additional TOML file for the final external replay. Native identities also retain executable paths, sizes, SHA-256 hashes, Rust/cargo versions and each exported fixture file's exact hash. Local wrapper/helper/executable identities sampled during packaging are in `runs/local-tool-identities.json.gz`. Existing warnings in MCP, the sidecar embedder and the overlay bench are retained; no warning cleanup was included.

[observations.json.gz](observations.json.gz) contains **37 actual packets**. They include complete Rust envelopes, supported capability digests, raw manifest bytes and parsed manifests, actual canonical paths and cache keys, `Arc::ptr_eq`, retained Rust snapshot contents, cold decoder errors and mutation requests. Ten publication packets capture both pointers' before/after bytes and raw canonical manifests. All six Rust mismatch cases reject with both pointers and the canonical manifest unchanged; four equal-envelope controls publish. The existing later pointer-I/O failure ordering test also passes unchanged.

The probes exercise Some(empty) → None → Some(empty), a valid Complete → declared Unknown envelope/checksum change at the same source hash, replacement canonical paths, valid header changes, 14 malformed declared header cases, and three unrecognized legacy schema spellings. Recognized v12 absence remains accepted. The Unknown envelope is an explicit stored test mutation, not a claim that the parser emitted Unknown. All fixtures use real temporary directories; no live graph pointers or native installs were used.

Capability digests identify declarations and do not validate them. The cache binds every parsed manifest field, rejects declared incompleteness before a hit, and on a miss validates decoded capability/counts and rereads the header before caching. A hot immutable Arc may survive table mutation with an unchanged declaration. The missing-table control preserves that hot snapshot and rejects a cold load at another canonical path. It does not claim continuous monitoring of table bytes. The integration suite additionally exercises valid-Parquet same-count mutations under an unchanged declared checksum.

Each `runs/*-native.tar.gz` contains replayable Parquet fixtures and raw packets. `transitions-red-native.tar.gz` captures the first 25 cache packets; `pointers-red-native.tar.gz` adds the ten corrected pointer packets to the same cumulative RED directory. Use the earlier archive's source identity for the earlier packets. `cache-green-native.tar.gz` is a fresh complete 37-packet capture. `native-replay-file-green-native.tar.gz` is the independent three-packet request replay; [replay-observations.json.gz](replay-observations.json.gz) preserves its results. It shows a retained present envelope, a new same-source absent envelope with a different Arc, and publication rejection against the original expected snapshot.

Recheck the packaged evidence without a build:

```bash
d6_evidence=docs/research/artifacts/2026-11-06-rust-evidence-durability/D6/worker-evidence
python3 "$d6_evidence/verify-evidence.py"
python3 "$d6_evidence/verify-native.py" "$d6_evidence/observations.json.gz"
```

The checks verify full PRE envelopes, command/log/source hashes, four complete native archive inventories, committed implementation bytes, 37 packet relationships and the three-packet replay. They also independently change a loaded capability claim, alias a replacement-path key to the previous key, and alter a rejected publication's pointer bytes; each incorrect claim is rejected. These are worker evidence checks, not ROOT's fresh solver POST. ROOT can call `verify()` on independently edited packet dictionaries or run the CLI on an edited observation file.

Required GREEN commands, with their actual arguments:

```bash
export SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0
scripts/spur-cargo test -p spur-graph store::cache::tests --config "$d6_evidence/native-green-env.toml" -- --nocapture --test-threads=1
scripts/spur-cargo test -p spur-graph --test rust_evidence_parquet --test integration_git_incremental --config "$d6_evidence/native-green-env.toml" -- --test-threads=1
scripts/spur-cargo check -p spur-graph --all-targets --config "$d6_evidence/native-green-env.toml"
scripts/spur-cargo fmt --package spur-graph --check
git diff --check
```

For a fresh native replay, extract the GREEN archive into a temporary fixture directory on the VM. Copy `runs/replay-request.json.gz` to a new request JSON and set `first_dir`, optional `next_dir`, optional `expected_dir`, `replacement_path`, optional `manifest_patch` (JSON-pointer keys), or `remove_table`. Use a TOML file to set `D6_REPLAY_REQUEST` and a fresh `D6_EXPORT_DIR`, then run:

```bash
scripts/spur-cargo test -p spur-graph --lib store::cache::tests::d6_native_replay --config "$d6_evidence/native-replay-env.toml" -- --exact --nocapture --test-threads=1
```

The committed config and exact request preserve this worker's tested VM paths. ROOT should substitute fresh temporary paths when independently replaying. The replay copies inputs into its own temporary common directory, retains the first Arc, applies the requested replacement/mutation, loads again, and optionally calls the real publication validator. [run.py](run.py) preserves exact commands and logs; [fetch-native.py](fetch-native.py) binds and retrieves native outputs. The completion audit ID is `d9daf871-6b62-4375-b48e-35464fda8073` on the canonical issue; approval remains ROOT's responsibility.
