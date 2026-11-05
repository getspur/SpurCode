# M2: shared local helper policy

Implementation is ready for root review on `bd-7c53`, delegation
`12e296d9eb054b09`, plan `dc9cc6fb-d54a-4429-a231-1f7f3425f7fd` / `shared-policy`.
The issue remains `in_progress`. Root's fresh actual-output Jev POST and node3
PRE are still required before approval. No approval or downstream dispatch is
claimed by this packet.

Milestones:

- Test commit `2d45a1c20`: shared configuration/adapter regressions and original assertion RED.
- Fix commit `2c01f064f`: neutral policy/configuration types and analyst configuration projection.
- This evidence commit adds actual observations and caller arithmetic, with no further production change.

`LocalPolicy::from_env()` uses the 14 reviewed defaults when the JSON variable
is absent. A supplied `SPUR_EMBEDDING_HELPER_CONFIG` retains the complete existing
schema, required fields and strict unknown/duplicate rejection. `HelperConfig`
and direct `LocalPolicy` use the same validation, including zero limits,
protocol row-size arithmetic, queue-plus-active overflow and checked deadlines.
Independent explicit frame/text/total/artifact caps may be below defaults.
Policy reading neither resolves an executable nor accesses model/native state.
Graph's adapter is deliberately deferred to node3.

The only analyst production changes are the policy import, removal of the old
private timeout deserializer, and `Timeouts::from_env()` projection. The verifier
compares every owner/job/completion/cancel/shutdown byte outside that adapter
to the dispatch baseline. Graph, neutral client ownership and worker/native
sources retain their pre-gate hashes.

## Authorization and numeric PRE

[entry-audit.json](entry-audit.json) contains the exact INTENT request and both
successful responses: fresh UUID `d7c1e8a6-4bf3-48c9-a5c7-01e248de6339`, comment
3405, identical replay `idempotent=true`. Live issue/plan checks established
the matching single delegation, assignee, `in_progress` and approved predecessor
before edits/builds. Git's initial index-lock EPERM required authorized
escalation to create the scoped commits; no authorization rejection occurred.

Root's [M2 PRE](../M2-pre/README.md) verifier passed before edits/builds against
the exact 40-file fingerprint
`baafa46271a6b66399c914df92d3b557add71cd93cb2312dfb16d743a5d49ae3`.
All six current receipts were reloaded through MCP and compared exactly with
the archived lowerings/status/model/Z3 identity; the reloaded objects are
[pre-live.json.gz](pre-live.json.gz). The independent root oracle checked
90 catalog predicates. Current resource/data-integrity catalog discovery was
also inspected. No retired M1 current-source checker was executed.

## Actual scoped results

All commands use `scripts/spur-cargo`, local `SPUR_REMOTE=0`,
`TMPDIR=/private/tmp`, target `/private/tmp/rust-gap-g1-acceptance-target`, and
one Cargo writer. Full commands, environment, exit codes and escalation flags
are in [commands.json](commands.json).

| Run | Observed result |
|---|---|
| Original default-policy assertion | RED, 0 passed / 1 failed; absent variable produced `NotPresent` |
| New analyst full-policy regression | RED, 0 passed / 1 failed; old projection accepted missing/unknown/invalid protocol fields |
| Neutral full suite, `test-support` | GREEN, 46 passed / 0 ignored |
| Final normal-feature policy suite | GREEN, 7 passed / 0 ignored |
| Analyst embedding unit scope | GREEN, 9 passed / 1 ignored / 105 filtered |
| Analyst embedding integration | GREEN, 29 passed / 0 ignored |
| Scoped local formatting | Exit 0 |

The two neutral runs overlap; these counts must not be added as distinct tests.
The analyst ignore is the existing explicitly configured real-helper/model-cache
acceptance test. The fixture integration suite covers Auto/Sidecar/Off,
`SPUR_EMBED_SOCKET`, no-launch behavior, v1 socket translation, cancellation,
publication, retained errors, shutdown and the native-free default product
dependency closure. It uses private temporary sockets and Python fault helpers.
No real model inference or default helper discovery is claimed by M2.

The first neutral sandbox run failed with process-inspection EPERM. Its full
[failure log](neutral-sandbox-eperm.log) is preserved; the authorized run above
passed. An extra arithmetic observation test initially expected the reader's
`FrameTooLarge` variant from a write. Its [failure](arithmetic-test-expectation-failure.log)
is preserved. Source inspection established that the existing writer returns
the cap failure through serde JSON/I/O. Only the test expectation changed; the
production protocol and defaults did not change. Existing graph (2) and analyst
lib-test (10) dead-code warnings remain visible in the logs.

## Actual policy and arithmetic for root POST

[policy-export.json](policy-export.json) is serialized by the normal-feature
Rust test from `LocalPolicy::default()`, rather than copied from the PRE proposal.
[policy-observations.json](policy-observations.json) contains 54 actual parse
inputs/results/errors: 6 accepted and 48 rejected. Cases include missing,
unknown and duplicate fields, invalid JSON/types, zero limits, queue/row
overflow, nonrepresentable u32 frame limits and valid independent small caps.
Separate child-scoped tests cover absent/explicit/invalid/non-Unicode environment
values and non-existent helper/cache paths. No test mutates global environment.

On this macOS arm64 / Rust 1.94.1 host, all four `u64::MAX` millisecond deadline
overrides are representable by `Instant::checked_add` and were accepted. The
tests preserve that platform-dependent contract; do not encode these outcomes
as overflow rejections in POST. Zero deadlines were rejected. Queue and row
arithmetic overflows and a frame value above `u32::MAX` were rejected.

[caller-arithmetic.json](caller-arithmetic.json) records the actual Rust framed
serialization: 64 × 65,536 raw UTF-8 bytes = 4,194,304 bytes, with all five free
identity fields filled to 64 control bytes and maximum request-id width. The
escaped request body is exactly **25,168,231 bytes**, within the default
33,554,432-byte cap. A one-byte-short cap returns `JSON: frame too large` and
publishes **zero** bytes. Raw 64 × 768 × 4 float storage is 196,608 bytes; that
is not a bound on response JSON or RSS. Declared first-request phase budgets
compose as 120,000 + 120,000 = 240,000 ms; these are budgets, not new latency
measurements.

The artifact budget 644,080,771 is the root-reviewed cap for the captured
five-file revisions. It does not certify future revisions. The 4,096-byte body
eligibility threshold does not bound titles, symbols or queries. Arbitrary graph
batch overrides and exact serialized-size admission remain later-stage work.
No solver optimality, future latency or native-memory proof is claimed.

## Source/build binding and replay

[source-binding.json](source-binding.json) binds 47 source files, compiled test
artifact hashes, raw logs and exports. Current fingerprint:
`4eba6245117b2d56fd1df9867a8c40b27ee7514c4266a386f6a848a86be1b895`.
The normal-feature policy binary and fixture-enabled neutral binary are recorded
separately. Historical RED binaries were rebuilt; their original binary hashes
are not asserted. Their source identity and exact failing logs remain preserved.

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M2/verify.py
```

This verifies current sources, compiled artifact identities, unchanged owner
bytes, exports against raw test output, root defaults, caller arithmetic and
the six reloaded PRE receipts. It is evidence preparation for root POST, not a
replacement for Jev or actual runtime tests.

Requested routing is `codex/profile codex/gpt-6.1-sol/xhigh`. Actual resolved
model is not exposed in this worker's tools/context or whitelisted model
environment; no model selection or fallback was performed. Root attestation was
requested through progress. Old notebooks, graph/CURRENT, main's dirty files,
TUI/xtask changes, external notebook sources, installed binaries and shared
model/runtime caches were not modified by this task.
