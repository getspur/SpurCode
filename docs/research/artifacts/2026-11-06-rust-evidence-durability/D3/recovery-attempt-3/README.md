# D3 attempt 3 authorization handoff

The worker stopped before recovery, source changes, tests or builds after its INTENT audit was denied. The exact request/rejection is preserved from worker commit bc64b3586. No implementation success is claimed.

The visible issue status and assignee matched the replacement delegation, but worker_evidence::authorize_worker_task additionally requires exactly its durable delegation label. The timeline records a late completion from expired attempt 2 after the replacement had already been dispatched. Local clear_dispatch_intent_update removes all current delegation labels. This supports a late-cleanup race as an inference; the running service binary revision and exact intermediate label write were not established.

Recovery preserves all implementation, ROOT's five failing regressions, the current supplemental PRE and both failed-dispatch records. The exhausted canonical issue will be retired without approval; a new canonical issue will start from this review branch, so it has the complete code/tests/evidence and cannot be modified by cleanup addressed to the old issue. The same selected-path repair scope, user-selected model, TDD and actual-output ROOT POST/review remain required. No authorization guard is weakened and no core/runtime code is changed.
