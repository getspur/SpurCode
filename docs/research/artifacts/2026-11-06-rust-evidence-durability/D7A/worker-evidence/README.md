# D7A API scaffold and TDD checkpoint

Canonical issue **bd-1sy1**, delegation `a51cbcdde45f4cae`, plan `77165a84-fa5c-4d7d-a185-042927cafbd4`, task `D7A-query-TDD-checkpoint`. Leave the issue in progress for ROOT review. This packet completes only the bounded checkpoint: **full D7 and D8–D10 remain incomplete**. No functional memory/Parquet query implementation, ROOT POST, approval, merge, push or issue closure is claimed.

## Source and entry

- Recovery base: `821b6a11207cc1437a06943dbb7a708267cc8548`.
- Original 508-line, eight-test commit: `3b943f783c51ca65169b49eacee8ad853fc4ab4a`.
- Scaffold commit: `40e5166a94f1070a19635b64b53f865045abc8f4`.
- Original test SHA-256: `09ffb8a2c2beb09a116dc5e9411a6777cdf4bd2b3751aae2e8c967473eae779e`. **Every test byte is unchanged; no setup/type correction was needed.**

The canonical issue was in progress, assigned to `spur-worker-a51cbcdde45f4cae`, with matching delegation, plan, task and base labels, and no supersession. INTENT UUID `56b5f729-5dc6-404f-90b1-a63e54e20449` was persisted as comment `4169`, then idempotently replayed with the same complete payload. `entry.json.gz` retains both full responses and both complete live solver envelopes.

Before restoring source, ROOT's PRE verifier passed: fingerprint `d89bed2e44a91498c88fbc76db7f1b21ac205a884726e8f8c6dd400fb1d7138b`, 1,959 bindings and nine independent predicates. Both reloaded envelopes exactly equaled the archived `reloaded` values: `sol_e9991d6ce40443ef` SAT/pass and `sol_8df86478f2aa4cac` UNSAT/fail. All 20 snapshot files passed byte-length and SHA-256 verification. Only the three frozen draft source files were restored, byte for byte, and committed before native execution. `scaffold.diff` is the zero-context unified source diff against the recovery base; the original context diff is preserved as `scaffold-context.diff.gz`. The first staged whitespace check flagged blank context lines in that saved diff; the evidence packet was corrected before the final checks. The PRE establishes historical classifications and prospective requirements; it does not prove runtime implementation.

## Native observations

| Exact case in `rust_evidence_queries` | Wrapper exit | Native counts | Observation |
| --- | --- | --- | --- |
| `zero_source_and_stale_oid_have_memory_disk_parity` | 101 | 0 passed, 1 failed, 0 ignored, 7 filtered | Runtime assertion at line 89: `Unavailable` versus expected `Complete`; no compiler error |
| `custom_default_is_unknown_domain_unavailable_and_shape_checked` | 0 | 1 passed, 0 failed, 0 ignored, 7 filtered | Default unknown-domain behavior and original filter-shape rejection pass |

The control's existing export printed `{"name":"custom_default","value":{"availability":"unavailable","source_domain":null,"files":[],"origins":[]}}`. The original seed has no export invocation and stops at its first availability assertion. Its later file-count, memory/disk parity and stale-OID checks were not reached. The six other feature tests remain unchanged and were not run; these filtered runs do not establish a full-suite GREEN.

Both commands used `scripts/spur-cargo test -p spur-graph --test rust_evidence_queries <exact-case> -- --exact --nocapture`, with `SPUR_REMOTE=1`, `SPUR_NO_LOCAL_FALLBACK=1`, `SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0` and `SPUR_CLOUD_BUILD_SH=/Users/kevintruong/project/SpurCode/scripts/cloud-build/build.sh`. Network escalation was requested before each remote invocation. Exact argv, environment knobs, timestamps, 803 pre-run crate/build-input hashes, unchanged-source checks and exit codes are in `results.json` and the compressed per-run records. Full stdout/stderr, including wrapper and native output, are in `seed-initial.log.gz` and `control.log.gz`. Logs stayed outside the checkout until both remote commands completed; the source tree was immutable throughout each run.

The normal wrapper auto-spun assigned builder `spur-builder`, instance `i-0a89b3e1aa5f8b6c6`, r8gd.2xlarge in ap-southeast-5a, Debian 12. Queue tickets were 384 and 385. Fresh-VM boot, upload and toolchain waits were not counted as compilation/test evidence. Read-only transfer diagnostics are preserved. No jobs, shared locks, sources or targets were deleted, and no helper code or infrastructure was changed.

Native toolchain: rustc 1.94.1 (`e408947bfd200af42db322daf0fadfe7e26d3bd1`), host `aarch64-unknown-linux-gnu`, LLVM 21.1.8. The wrapper recorded `-j8` and `-Ctarget-cpu=neoverse-v2 -Ctarget-feature=+lse -Clinker=clang -Clink-arg=-fuse-ld=/mnt/cargo/rust-lld-driver/ld.lld`.

Both runs used `/mnt/cargo/targets/spur/worktrees/9285bbcb-cf45-4128-9a0c-797dcf038af0/debug/deps/rust_evidence_queries-18cb592f75401a36`. Read-only SSM probes before and after the control confirmed executable SHA-256 `5b7d5e950d51064884120903c26c90c0044ed9ad34bf0cb1691e6be93e1b1248` and all four source hashes. Full requests, responses and command argv are retained. The first login-shell probe returned success with empty output; it is retained as `identity-login-*` and supplies no identity evidence. Noninteractive probes produced the recorded native identity.

Both builds emitted two existing dead-code warnings (`FileOidMatch::as_bool` and `SymbolEmbedder::new`). The control log also contains cumulative sccache statistics; their historical compilation-failure count is not a compiler failure in either observed test command.

## Inherited evidence and replay

`inherited/` preserves the original authorized compile RED (exit 101, 32 missing-API diagnostics), its pre-scaffold source hashes, and the failed assertion attempt (exit 255, AWS endpoint failure before compilation). The exit-255 attempt is **not runtime RED**. Its original inventory and the unchanged original test bytes are retained. Historical evidence belongs to its historical execution; this worker's audits target only bd-1sy1.

Run from this checkout:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7A/worker-evidence/verify.py
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7A/worker-evidence/replay.py /private/tmp/d7a-replay
```

`verify.py` checks packet hashes, complete PRE envelopes, INTENT persistence, original test bytes, bound source hashes and both native transcripts without network access. `replay.py` checks the bound scaffold and cloud-helper hash, then reruns exactly these two cases with the same argv/knobs and validates runtime RED/control GREEN. In the managed sandbox, invoke replay with network escalation up front; keep output outside the checkout. ROOT's PRE verifier is intentionally a pre-restoration check and should not be rerun against scaffold-modified production files as though they were the PRE state.

Scoped local formatting, diff checks and packet verification are recorded in `checks.json`. ROOT owns fresh actual-output POST and review, followed by a new implementation PRE. This checkpoint must not be used to approve the full feature.

Completion UUID `27e08f7a-5d7e-426a-964d-90cac098883b` was persisted as comment `4178` and idempotently replayed; the complete returned audits and payloads match exactly. `completion-audit.json.gz` retains both full responses and the request. Its inventory hash binds `checkpoint-inventory.json` and evidence commit `824a8fd9d9cce6345cbadc9b94b102690224f382`; the final inventory additionally covers the audit replay files. Canonical issue bd-1sy1 remained in progress with the same assignee and delegation.
