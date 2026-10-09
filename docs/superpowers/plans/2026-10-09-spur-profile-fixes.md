# SPUR profile fixes implementation plan

> Recorded under beads `bd-3ebxc`; the user approved direct implementation.

Source specification: `/Volumes/Projects/spur/spur-performance-rust-poc-20261009.ipynb`, approved POC task `bd-z4x5h`.

Goal: eliminate whole-history work on a warm stream frame and per-issue SQL reads during graph snapshot loading.

The implementation has two independent scopes, executed sequentially without worker dispatch:

1. **Renderer** — `crates/spur-tui/src/components/react_trace/{mod.rs,body_cache_tests.rs}` and `stream_pane.rs`. Tests first: cached lines share storage, the solver's 65,536-row scroll witness renders the correct row, append/resize/tick invalidate correctly, and placeholders retain wrapping. Return a borrowed slice from the generation/width cache, copy the visible range only, and remove Paragraph's truncating scroll cast. Preserve the existing follow/clamp behavior.
2. **Dependencies** — `crates/spur-pm/src/beads_crate/dependency_compat.rs`. Test full-field equality with the per-ID reference for duplicate/missing IDs and mixed types, and require malformed dependency records to surface as errors. Use the pinned backend's existing `get_all_dependency_records()` once and retain requested source IDs. Empty input performs no query. The API scans all dependency records; this is appropriate for the full graph snapshot caller but not a claimed improvement for every tiny filtered request. Unlike the old per-ID API, the bulk API propagates row decoding failures, including malformed records outside the selected IDs.

PRE: Jev 1.13.0 gate open (weakest confidence 0.94), custom rows/SELECT work budget verified by `resource.request_within_limit` (`sol_47fac730ee694147`). Generic slice counterexample unsat (`sol_105757bdeb4b43ad`); u16 counterexample sat (`sol_342b9223981340c2`). Experimental viewport sizes are fixtures, not new production limits.

TDD sequence: add runtime regression tests, run them with `scripts/spur-cargo`, confirm behavioral failures, commit `test(...)`, implement the minimal changes, rerun targeted and crate regression tests, run formatting/lint checks, execute Solve POST for the implemented slice/work budgets, review the diff, and commit `fix(...)`.

Validation commands:

```sh
scripts/spur-cargo test -p spur-tui --no-default-features --lib body_cache_tests
scripts/spur-cargo test -p spur-pm --lib dependency_compat
scripts/spur-cargo test -p spur-tui --no-default-features --lib
scripts/spur-cargo test -p spur-tui --no-default-features --features markdown --lib components::react_trace
scripts/spur-cargo test -p spur-pm --lib
scripts/spur-cargo fmt --all -- --check
scripts/spur-cargo clippy -p spur-tui -p spur-pm --no-default-features --lib -- -D warnings
```

The build/test catalog returned no applicable approved build workflow; routing follows repository AGENTS.md and the standard wrapper. No dependency updates, process restart, unrelated animation/image changes, or whole-app speedup claim are part of this patch.
