# M3 entry gate: helper discovery and child cache configuration

Root executed this source-bound PRE before approving M2. Canonical issue`bd-2fkd`, plan task`discovery-cache` in`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. Keep the existing explicit routing`codex/profile codex/gpt-6.1-sol/xhigh`.

Before editing or building, verify the new checkpoint and reload its seven current receipts:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M3-pre/verify_pre.py
```

Required source47 fingerprint:`4eba6245117b2d56fd1df9867a8c40b27ee7514c4266a386f6a848a86be1b895`.

| Case | Receipt | Expected |
|---|---|---|
| Discovery requirements |`sol_98b0870dfdd44179`|SAT|
| Compatibility requirements |`sol_20c0de74d4f94f52`|SAT|
| Cache/integration requirements |`sol_d62a183913834802`|SAT|
| Inherited numeric bounds |`sol_14e1a50f15e7462b`|SAT|
| Missing explicit helper falls back |`sol_1986b25716424642`|UNSAT|
| Wrong protocol accepted |`sol_63a2b669dbef4980`|UNSAT|
| Parent environment mutated |`sol_53205cfec76a43bd`|UNSAT|

Four Jev1.14.0 interpretations opened with weakest confidences0.88,0.95,0.94 and0.99 respectively. Root inspected exact facts/subjects/mode/no-unknowns and completed the catalog uniqueness/cardinality bindings. The7 persisted/reloaded requests cover24 declared cases,19 compound predicates and4 numeric comparisons. This is a requirements PRE, not evidence that M3 works. `requirements.json` and the three typed request tables are authoritative finite case projections; their labels describe behavior, not mandatory Rust API names.

Root also built the normal helper with current M2 source, no test features and non-test profiles, then copied it into a private path containing spaces. Direct command invocation returned exactly64 metadata bytes with empty stderr, version1.24.0/protocol2/macos/aarch64. Binary SHA256:`2c47fc09b3c9ef38ed09fd5dbd91042f1232900f7b06f97ead8dbe9dac0fa951`. The driver supplied child-scoped SPUR_ORT_CACHE and an unusable model cache; metadata created no native cache directory and left the parent's cache variable unchanged. This grounds the command and platform facts only. Sibling selection, graph cache forwarding and a real default model cycle still need implementation/tests. The frozen normal helper is retained at the path in`normal-metadata.json`.

## Implementation contract

Consume M2's validated shared policy in the graph loader and remove the mandatory-JSON path and duplicate private policy deserializers. Keep complete explicit JSON compatible and higher priority than the new executable override: preserve its program and prefix arguments, including explicit interpreter commands. Invalid supplied JSON fails without falling back. Legacy JSON command fixtures need not implement the new metadata command; keep their existing framed descriptor/Hello validation. This exception preserves existing override behavior, not an exemption from wire identity/correlation checks.

Without JSON, select explicit nonempty absolute`SPUR_EMBEDDING_HELPER` via`var_os`, otherwise the platform-named helper sibling beside`current_exe`. Do not search PATH for automatic discovery, invoke a shell, fetch/compile an executable at runtime or overwrite an installed binary. Missing/relative/invalid explicit paths fail without a sibling fallback. Preserve spaces, Unicode and Unix non-Unicode paths through`OsString` and argument vectors.

For new sibling/env selection, run only`--compatibility` before model/native work. Validate zero exit, strict JSON fields and exact package version, protocol, OS and arch against compiled constants. Return typed missing/path/metadata/platform/version/protocol/exit errors. Bound stdout using the already-approved`LocalPolicy.limits.max_frame_bytes`; use a checked over-limit sentinel count, not an unchecked u32/usize cast. Do not collect unbounded stderr. Preserve cancellation and cleanup ownership on a stalled, oversized, malformed, failing or aborted probe.

Compatibility and descriptor work share the effective caller/startup deadline; do not renew a full deadline between subprocesses. Reuse the existing startup timeout, cleanup polling and owner semantics. The actual64-byte metadata fits the inherited33,554,432-byte frame cap. The inherited120,000ms startup budget and20ms poll/30,000ms cleanup budget are configurable budgets, not new latency guarantees. No new numeric defaults are authorized by this gate; send a concrete supplemental request if implementation needs another bound.

Pass an immutable child environment to descriptor and serving commands. Preserve a nonempty explicit`SPUR_ORT_CACHE` as an`OsString`; relative values retain their current-directory meaning (capture/resolve that base consistently if needed). Otherwise derive`HOME/.spur/cache`, falling back to`XDG_CACHE_HOME/spur` when HOME is absent; require a valid absolute base and report a typed failure if none exists. Reject empty/invalid cache input rather than silently selecting another directory. The worker's existing runtime alone appends`onnxruntime/1.22.0/<pinned SHA>/libonnxruntime.dylib`; graph/policy/probe must not create native directories. Never use global`set_var`, mutate model/cache/profile identity or change the native materializer. Both descriptor and serving commands must receive the same resolved child value.

Keep the already-reviewed State/Gate/job/publication/cancel/shutdown mechanisms, cleanup owner, Hello validation before native startup,768 dimensions, max-length8192 and existing model prefixes/profiles. Only immutable command/environment configuration and the graph adapter are in scope. Minimal analyst configuration glue is allowed by the task, but no ownership refactor or new upstream audit. Off, disabled/empty inputs and unrelated socket modes must still avoid launching the helper.

## TDD and acceptance

Capture assertion REDs for sibling/env selection, exact path handling, missing/wrong metadata and absent child cache, then commit the minimal implementation and scoped GREEN. Test malformed/oversized/nonzero/timed-out/cancelled probes, checked size arithmetic and cleanup using private fault helpers. Preserve existing complete JSON fixtures and modes. Use child-scoped environment changes and no shared target helper substitution.

Build the ordinary helper explicitly and exercise the actual graph/client path from a private temporary installation with spaces, without mandatory JSON. Use a private native cache and private copies of the already available model assets; existing shared caches are read-only. At least one real Nomic lifecycle must produce finite768-dimensional output and close with checked child exit0, with command/source/native/model hashes recorded. This establishes a scoped working cycle; three-model comparisons, broad fault containment, batching, shipped packaging and Notebook acceptance remain later gates.

Use only`scripts/spur-cargo`, local SPUR_REMOTE=0, TMPDIR=/private/tmp, target`/private/tmp/rust-gap-g1-acceptance-target`, one writer. Native archive and DuckDB extension paths remain those in the original task. No root Cargo writer will remain when this task releases. Preserve user/main/notebook/graph state, global installations and caches. Commit tests, fix and evidence milestones, report_progress at yields, and leave the issue in_progress for root's fresh actual-output POST and M4 PRE. Write observations to`docs/research/artifacts/2026-11-05-embedding-sol/M3/`.

Do not run old current-source PRE verifiers or require retired M1 receipts. Historical source/proof objects remain committed. The live entry gate is this M3 checkpoint; all seven receipts above stay pinned for dispatch. Other-platform and non-Unix execution are not claimed by the grounding run.
