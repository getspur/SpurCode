# Sol implementation continuation

The user selected **codex / profile codex / gpt-6.1-sol / xhigh** for all remaining
work. The prior stage exhausted its attempts with tests and evidence only; no
production implementation is accepted. Its final test checkpoint
`c050a4650741fd3963a0df4bc0ac4adc6433bf23` is preserved and provides the next
implementation baseline outside local main.

The new 40-file source fingerprint is
`679f59599671855f52143e1809b7ac81525e8aa30c53ffa9f777df8026ba1fac`.
Compared with reviewed source `091bb89af`, changes are confined to the analyst's
cfg test module and a worker dev-dependency. The verifier checks both that
distinction and every current source hash before implementation.

The first focused task implements the existing failing `--compatibility` test:
normal-feature JSON metadata using Cargo version, existing protocol constant and
std target OS/arch, before model or native initialization. Shared configuration,
discovery/cache propagation, bounded batching, packaging and actual Rust notebook
validation remain separate reviewed stages. Completing metadata does not complete
the original shared-defaults stage.

Eight persisted/reloaded compatibility PRE receipts have one SAT and seven
expected UNSAT outcomes. Four further receipts complete the worker's previously
blocked numeric check: captured row counts, declared identity dimensions and
response-frame sizes fit the experimental fixture bounds; three boundary
mutations are rejected. The verifier checks all 28 catalog predicates independently,
including all nine integer comparisons inside each fixture predicate.
These checks do not select product defaults or certify vector values, future
capacity, fresh inference, or raw POSIX wait statuses.

The initial redundant eight-row metadata model exceeded the 64-variable limit.
The executed three-field model preserves all cases with 56 variables. A broader
Jev wording was blocked; the executed request explicitly verifies only complete
finite declared requirement tuples. No blocked request was treated as an open gate.

Solver archive capacity was restored through explicit receipt lifecycle actions.
Full committed archives for two completed root reviews were hash-verified:
17 live receipts were unpinned; 30 were already unavailable. Ten older PRE
receipts for the exhausted source-091 stage were also verified in Git and
unpinned. Their exact evidence remains archived. Current metadata/fixture
controls remain pinned; the global quota was not increased.

Run before edits or builds:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol-continuation/verify_pre.py
```

Each worker must persist and replay an authorized INTENT audit, keep MCP
heartbeats during long commands, observe assertion RED, implement, run scoped
GREEN and await root's fresh POST. Root supplies each successor's PRE before
approving its predecessor. Existing user edits, notebooks, caches and installed
binaries remain unchanged. The separate dispatch fencing bug `bd-mbta` remains open.
