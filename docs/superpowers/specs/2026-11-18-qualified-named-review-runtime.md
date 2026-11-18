# Qualified named runtimes for Spur Review

Implementation is authorized by the request to solve the remaining issues with Rust/Python notebook experiments before TDD. This specification carries the prior six-gate findings into source changes. Epic: `bd-3oku`.

## Grounding before production tests

Live proof notebook: `/Users/kevintruong/project/spur-notebook/2026-11-18-named-runtime-tdd-proof.ipynb`. Captured proof: `docs/research/artifacts/2026-11-18-named-runtime-tdd/proof.json` and `proof-notebook.ipynb`. Source base: external `spur-notebook` commit `b5885ef86b939680b7803c236ee81005beac8aa9`.

The Rust routing model changes only opaque named Deno startup, preserving factory Deno and existing Python/C++/Rust behavior. A second Rust model rejects identity mismatches, absent carrier, cancelled/no-longer-Running work and unavailable qualification. These are source-grounded models; actual source tests remain required.

The actual named Python interpreter experiment demonstrates a ContextVar carrier preserving async A, while a new unbound thread has no carrier despite its latest parent reporting B. A modeled current host receipt admits B and rejects late A and the unbound thread. Catalog tuple consistency passes the proposed policy and rejects admission inferred from the latest parent. P0 `bd-20hj` is approved before implementation tasks start.

## Required behavior

### Deno routing

Named Deno cold ensure defers to guarded dispatch, which owns launch and immutable ownership binding. Explicit and inherited/default lifecycle calls resolve the canonical saved target. Factory Deno remains a legacy target returning no native target. Foreign notebook, unknown/superseded UUID, kernelspec mismatch and incompatible current slot ownership remain rejected. Enumerating factory Deno through a closure expecting a native target is forbidden.

### Qualified Python ports

A named Python profile gains ports only through a qualified process registration and actual interpreter/IPC capability. Missing prerequisites remain explicitly unavailable. No factory filesystem helper, port-root environment inheritance or implicit package installation may enable the path.

Host origin is bound before execution from immutable dispatch input; the exact Running receipt exists before code can publish. A carrier must identify the original execution even for async work. Unbound threads do not gain the latest cell's authority. A correct old origin is still rejected once its admission is no longer current.

The canonical publication fence checks notebook identity, selected profile UUID, process generation/registration, input fingerprint, run ID, source/version and lifecycle/cancellation. Python put/get/IPC operations use host admission, atomic commit and receipt/recovery semantics. Nulls, order, schema and empty batches are preserved. Malformed IPC or a lost reply must not create an unowned or duplicated publication.

Binding must preserve IPython source behavior: display of last expressions, top-level await, magics and useful source coordinates. Per-request metadata and a serialized pre-run hook are preferred to arbitrary textual prefixes. The integration test must exercise the actual existing interpreter rather than infer readiness from a version string.

### Deno consumption and app seam

Named project Deno uses its asynchronous v2 API. A qualified named Python producer must publish a versioned Arrow port that named Deno reads using `await spur.get` or `await spur.getIPC`. Replacing source data reruns the view while retaining selected review context. A delayed superseded run cannot update the displayed snapshot. Stateful anywidget/helper composition must be qualified explicitly; successful startup alone is insufficient.

### Collector and tool contracts

An unchanged saved API connection attaches without losing provider/credential reference or rewriting credential values. A changed authenticated manifest must not silently reuse a grant for a different origin. Notebook save advertises structured Jupyter cell objects matching its actual handler and preserves empty-overwrite protection.

The collector accepts one explicit repository/PR request, pins immutable base/head/repository identities, inventories all tracked files, and retains typed graph/provider coverage. Current PR metadata, history and graph context are joined into one finalized review packet. Candidate links remain candidates; missing paths do not mean no impact. Moved-head or partial-provider results cannot silently mix snapshots. Existing DuckPGQ grammar activation and reference app doctor behavior remain covered.

## Verification and delivery

Each source task produces a meaningful failing-test commit before implementation, then a minimal fix commit and relevant green regressions. Rust/frontend builds use repository wrappers with remote execution; no local fallback may hide a remote failure. Source work is isolated from user-edited notebooks.

Integration must distinguish source tests, existing-interpreter protocol tests, a fresh patched binary and the currently installed app. Tests do not imply the installed app has changed. A candidate binary may be tested in an isolated process/profile; do not replace or terminate the user's running app. No packaging, publication or deployment is required.
