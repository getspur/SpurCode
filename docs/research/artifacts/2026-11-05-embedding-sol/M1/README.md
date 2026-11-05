# M1: normal-feature compatibility metadata

Implemented and scoped GREEN; awaiting root's fresh actual-output Jev POST and
review. Issue `bd-1l17`, delegation `29e78737d3a04a16`, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. The issue remains `in_progress`.
This result does not establish broader default-helper behavior.

The production change is confined to the worker's `Cargo.toml` and `src/main.rs`:
make `serde_json` a normal dependency, preserve the `test-support` feature, and
return metadata before protocol limits, model resolution or native startup.
Any invocation combining `--compatibility` with other arguments fails before
those branches. Version, protocol, OS and architecture use existing constants.
No helper configuration, defaults, batching, timeout or model limit changed.

## Authorization and RED

- Base: `c4253c5b704d8fc302d8ca4452b5fe4a26079fbe`, initially clean worker worktree.
- Existing test checkpoint: `c050a4650741fd3963a0df4bc0ac4adc6433bf23`.
- The required continuation PRE verifier passed before edits/builds: 40 source
  hashes, fingerprint `679f59599671855f52143e1809b7ac81525e8aa30c53ffa9f777df8026ba1fac`,
  eight metadata receipts, four fixture controls and 28 catalog predicates.
  All eight live metadata receipt statuses matched. See [pre.json](pre.json).
- INTENT audit `89df38be-1948-4c2c-b185-bf4748a347d9`, comment `3383`, persisted
  and identical replay returned `idempotent=true` before edits/builds. Status,
  sole delegation label and matching assignee were checked on the fresh issue.
- [compatibility-red.log](compatibility-red.log): original existing test compiled
  and failed at its assertion with `expected seven protocol limits and artifact budget`.
- Test/evidence commit `2fb09db42`; [committed-tests-red.log](committed-tests-red.log)
  then observed the same assertion RED (1 failed, 1 passed). One table test covers
  invalid combinations; the original test also checks clean stderr.
- Fix commit: `2c3c13dd1`.

Git initially hit sandbox EPERM creating this worktree's index.lock under the
parent repository's Git directory. The scoped commits succeeded with authorized
escalation. No process/socket environment failure was counted as GREEN.

## Scoped verification

Commands, environment, timestamps and exits accompany every log in matching JSON.
All Cargo commands use `scripts/spur-cargo`, local execution, one writer, the
authorized target directory and pinned archive. Formatting is local.

| Evidence | Actual result |
|---|---|
| [scoped-green.json](scoped-green.json), [log](scoped-green.log) | 10 passed, 0 failed/ignored: 2 unit, 2 compatibility, 1 descriptor-client, 5 fixture metadata |
| [all-targets.json](all-targets.json), [log](all-targets.log) | `check --all-targets --all-features --locked`, exit 0; includes support binaries |
| [format.json](format.json) | Crate-local `fmt --check`, exit 0 |
| [normal-build.json](normal-build.json), [log](normal-build.log) | Separate `build --bin spur-embedding-worker --no-default-features --locked --message-format=json`, exit 0 |
| [normal-output.json](normal-output.json) | Both worker and neutral-crate artifact feature lists `[]`, `profile.test=false`; actual command exit 0 |
| [argument-baseline.json](argument-baseline.json), [after](argument-after.json) | Five exact matching pre/post exit/stdout/stderr results: no args, unknown flag, incomplete describe, invalid limits, serve EOF |

Six real-model/network metadata tests were explicitly filtered from scoped GREEN;
their exact names are in the command JSON. Other inference/native lifecycle tests
were not run. Existing vendor ORT and worker runtime warnings remain in logs.
Verification was on macOS arm64, dev/unoptimized profile; no other platform claim.
Subprocess exits are Python return codes, not raw POSIX wait-status words.

The freshly built normal binary was externally executed as:

```sh
SPUR_MODEL_CACHE=/dev/null/unusable SPUR_ORT_CACHE=/dev/null/unusable \
  /private/tmp/rust-gap-g1-acceptance-target/debug/spur-embedding-worker --compatibility
```

Raw [stdout](normal-compatibility.stdout), with empty [stderr](normal-compatibility.stderr):

```json
{"arch":"aarch64","os":"macos","protocol":2,"version":"1.24.0"}
```

A separate invocation with `DYLD_PRINT_LIBRARIES=1` captured a nonempty
[loader trace](dyld-trace.stderr), with no `onnxruntime` match. Together with
the early-return source and unusable caches this corroborates no native startup
for this command. Success with unusable paths does not prove absence of every
filesystem read or general resource ownership.

Normal binary SHA256:
`dc4628e3639df6eb7d73676865ed6afa70c0483678310746414938a278e42122`.
Updated 40-source fingerprint:
`baafa46271a6b66399c914df92d3b557add71cd93cb2312dfb16d743a5d49ae3`.
[source-binding.json](source-binding.json) records every source hash, additional
test/build-source hashes, binary hash, archive hash and embedded runtime dylib hash.
Exactly the worker manifest and main source differ within the PRE's 40-file set.

`python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M1/verify_evidence.py`
verifies these current bindings and independently preflights the finite root
candidates. [artifact-verification.json](artifact-verification.json) records exit 0.
An earlier import-path error is preserved in
[candidate-preflight-import-failure.log](candidate-preflight-import-failure.log).
That evidence QA error was corrected without production changes.

## Root handoff

[root-post-candidates.json](root-post-candidates.json) supplies a complete finite
catalog candidate grounded in the captured output, source/target constants,
normal compiler artifacts, baseline comparisons and loader trace. Nine synthetic
negative controls cover limits-required exit, stderr contamination, wrong
protocol, test-only build, native loading, changed serve errors, missing target,
duplicate case and unbound source. These are candidate mutations, not observed
runtime failures. Root must freshly compile Jev POST, inspect its binding, execute
and persist/reload the catalog checks, then prepare node2's concrete policy PRE
before approval. No worker self-approval or downstream dispatch occurred.

Requested routing is `codex / profile codex / gpt-6.1-sol / xhigh`. Worker MCP and
plan status confirm the Codex agent and delegation, but expose no exact resolved
model/effort attestation. The worker performed no model selection or fallback;
root must attach the dispatcher attestation for exact routing.

Dirty main files, historical/live notebooks and graph/CURRENT, global caches and
runtimes, installed binaries, TUI/xtask edits and external notebook sources were
not modified. No push, publication or service restart occurred.
