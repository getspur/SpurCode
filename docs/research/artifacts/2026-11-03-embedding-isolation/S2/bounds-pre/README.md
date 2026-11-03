# S2 supplemental root PRE

The test-only S2 submission is preserved at candidate
`474011fc0d281ce68fe872ea8e033d4421acc8ca`. Its API compile RED is genuine;
no supervisor assertions or fault fixture processes have run yet. S2 is not
approved. This supplement resolves the requested PRE dependency so production
TDD can continue.

`manifest.json` records the exact archive checksum and 13 solve IDs. The
byte-preserved requests, Jev provenance, typed preflights, results and reloaded
lowered requests are in `full.json.gz`. Run `python3 verify.py` from this folder
to check the local archive and its historical source bindings. This verifier
does not replace fresh `get_solve_result` lookups by the worker.

Two fresh Jev interpretations opened and were inspected: the resource request
limit and the five-step cleanup transition relation. Eight catalog results
check declared queue/inflight/correlation/spawn counts and one prospective
cleanup order, with six rejection controls. Family receipt reloads contain the
lowered generic predicates; the original family requests are retained too.

The catalog does not express absolute-deadline arithmetic or the guarded
restart-counter recurrence. Five exact typed requests passed preflight before
execution: deadline feasibility, no deadline reset, a timeout-reset overrun
witness, an expired-deadline counterexample search, and a restart-counter
counterexample search. The timeout-reset witness is SAT at queue delay 1 ms
and request timeout 1 ms, finishing at 2 ms. The other three counterexample
searches are UNSAT in their explicitly bounded domains. These are prospective
policy checks, not measured Rust timing or synchronization proofs.

The finite inputs come from the fixture policy: queue capacity 1, one in-flight
request, four protocol pending slots, one allowed restart, 2000 ms phase policy
and 3000 ms caller deadline. Symbolic restart budgets cover 0 through 64 and
time arithmetic covers the ranges in the manifest/archive. These ranges do
not become product defaults or product maximums. There is no proof of
unbounded liveness, all Rust integer widths, native inference, or future
implementation correspondence.

S2 must retain one absolute effective deadline from method entry, including
queue and startup waits. Validate caller parameters and checked time/counter
arithmetic. Keep queued admission separate from protocol pending registration;
this PRE does not impose `queue_capacity + 1 <= max_pending`. Charge restarts
before spawning and never silently replay uncertain work.

Cancellation must wake blocked callers, while cleanup must terminate the owned
child's IO before joining blocked reader/writer work, reap the child and close
pipes. The exact graceful order may differ from this one conservative forced
termination trace; preserve actual events for POST. Caller completion deadlines
and eventual OS reaping are separate obligations. A valid committed response
survives a later child abort. A correlated remote error remains an operation
failure.

The worker must now implement the missing client and obtain meaningful GREEN
runtime evidence. Preserve the test commits, add needed test-first startup,
IO-blocking, cancellation, shutdown and invalid-policy scenarios, and export
typed outcomes, monotonic timestamps, PIDs, request/generation identities,
spawn counts, child wait statuses and cleanup events. Root will execute fresh
Jev POST from those actual observations before S2 approval. Worker-local Jev
absence is already handled by the authorized root bridge.
