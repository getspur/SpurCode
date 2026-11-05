# Embedding continuation after assignment-race grounding

The packaging handoff is incomplete. Its PRE verifier passed, but the added
default-policy test was interrupted with exit 130 before an assertion RED.
No production implementation or GREEN is accepted from that attempt.

The Beads event history records the current dispatch label being added at event
7880, then removed at event 7893 immediately after an older worker completed.
The current worker started later and had the matching assignee, but the evidence
service requires both the exact delegation label and the current durable dispatch.
`clear_dispatch_intent_update` removes every delegation label in the supplied
label list, which matches the observed cleanup sequence. The underlying fencing
bug is reported separately as **bd-mbta**; this research does not fix that code.

The operational recovery uses fresh canonical task IDs after all old callbacks
completed. It preserves strict worker authorization and requires a successful,
reloaded `report_audit` before implementation. It does not reuse the exhausted
issue or treat the worker-started audit and assignee as sufficient authorization.

Preserved test milestones:

- `726ea20780f51c8935f2bb3ead608e0ea9f07188`: npm helper install layout regression.
- `49e855e8e80757aa361b5c9323fffc593f53db01`: default analyst policy regression;
  its assertion RED still needs to be observed.

The remaining pipeline separates shared helper discovery/configuration from
normal artifact packaging, followed by the original runtime and new Rust notebook
acceptance gates. Every stage retains TDD and fresh root Jev PRE/POST review.
Codex/profile codex/model gpt-6.1-terra/effort xhigh are passed explicitly in the
replacement plan. The implementation must pass a child-scoped native cache path:
the worker reads `SPUR_ORT_CACHE`, and default helper discovery alone is insufficient.

`pre.json.gz` contains the executed stage-A Jev request and catalog definitions,
ten complete solver requests/responses/reloads, and four separately attributed
historical budget controls. Stage A has one SAT and nine expected UNSAT results.
The verifier independently checks 34 catalog predicates and the current 40-file
source binding. These are PRE requirements, not new runtime or performance claims.
Numeric product defaults still require caller/measurement grounding and concrete
catalog checks before implementation; recorded fixture timings do not select them.

After artifact quota exhaustion, only nine superseded parent-contract receipts
were unpinned. Their full original evidence remains committed at `004938881` in
`2026-11-05-embedding-main-continuation/pre.json.gz`. The ten new stage-A receipts
and four historical budget controls remain pinned; no global cache policy changed.

Verify before edits or builds:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-assignment-recovery/verify_pre.py
```

Existing dirty ACP files, live notebooks, global runtime/model caches and the
external notebook repository remain untouched. No installed binary was overwritten.
