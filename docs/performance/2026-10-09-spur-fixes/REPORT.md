Implemented the approved notebook proposal in project `spur`, tracked as `bd-3ebxc`.

The stream renderer now borrows the generation/width cache and copies only visible rows into `Paragraph`. It no longer clones or rewraps the complete warm history. Slicing with `usize` also fixes the observed offset-65,536 wraparound. The cache's existing invalidation and the pane's clamp/follow behavior remain in place.

Unfiltered graph snapshots use the pinned beads_rust backend's existing `get_all_dependency_records()` API, replacing one dependency SELECT per issue with one full-record SELECT for nonempty snapshots. Empty input performs no dependency query. Label-filtered snapshots retain scoped per-ID reads: review caught that using a full-table scan there would regress small requests and expose them to unrelated corrupt rows. Full snapshots now report malformed dependency rows instead of silently omitting them; this includes malformed rows belonging to excluded sources, such as tombstones. Filtered snapshots retain the legacy backend's handling of their own malformed rows.

The backend is pinned at `47b3b39a54b04c7d51d735c032c82e891c2a0db8`. Its native batch query selects all seven dependency fields, preserves per-source target ordering, and propagates row conversion errors. No dependency version or schema change was needed.

TDD recorded behavioral failures before production changes:

| Regression | Observed RED | Test commit |
|---|---|---|
| Corrupt dependency row | Per-ID API silently omitted the edge; expected error assertion failed (3 passed, 1 failed) | `46ac7a599` |
| Cache storage reuse and long scroll | Returned a copy; offset 65,536 displayed row 0 (3 passed, 2 failed) | `08efbbcbf` |
| Filtered snapshot isolation | Unrelated invalid timestamp caused the selected snapshot to fail (0 passed, 1 failed) | `f7bf97545` |

The TUI regressions exercise the real ReactTrace and renderer: storage reuse, solver-derived long scroll, 192 framebuffer/state cases, 16 placeholder cases, and cache invalidation after append, resize, spinner tick, and clear. Dependency tests use real SqliteStorage, compare every field, and cover duplicate/missing IDs, empty input, and malformed records.

Final runtime verification on the remote AWS builder:

| Command (through `scripts/spur-cargo`) | Result |
|---|---|
| `test -p spur-tui --no-default-features --features markdown --lib` | 1,328 passed; 0 failed; 4 ignored |
| `test -p spur-pm --lib` | 210 passed; 0 failed; 0 ignored (`pm-green.log`) |
| `fmt --all -- --check` | Passed |
| Remote `clippy -p spur-tui -p spur-pm --no-default-features --features spur-tui/markdown --lib -- -D warnings` | Blocked by pre-existing unused `FileOidMatch::as_bool` in `spur-graph/src/mcp/mod.rs:908` (`clippy.log`) |
| Same Clippy command with `--no-deps` | Blocked by pre-existing `clippy::unused_async` in `spur-tui/src/app/analytics.rs:342` (`clippy-scoped.log`) |

Both lint-error files are unchanged from baseline `982464734`; no diagnostics were reported in the changed code. These checks are recorded as failures, not waived passes. An initial wrapper-default local lint attempt was canceled during dependency compilation and rerouted with `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1` because local disk space was low.

The initial TUI run without markdown did not reach tests: seven pre-existing test initializers in `compact_render.rs` set the feature-gated `markdown` field. This compile failure was not counted as RED; the markdown-enabled run produced the intended behavioral failures. Full default-feature/binary builds were not part of this focused library validation.

Jev/Solve evidence is saved as exact requests and receipts in `solve-pre.json`, `solve-scope-pre.json`, and `solve-post.json`:

| Check | PRE | POST |
|---|---|---|
| Slice safety: search for an out-of-bounds range | unsat `sol_105757bdeb4b43ad` | unsat `sol_efb096d27ab74ea6` |
| Old u16 truncation witness | sat `sol_342b9223981340c2` | Covered by passing production regression |
| Warm viewport / dependency SELECT budget | pass `sol_47fac730ee694147` | pass `sol_ea6c2451bfa148cf` (unfiltered batch only) |
| Filter and bulk scan exclusion | pass `sol_4e33f6219eb64192` | filtered pass `sol_f02d0780862e4fd2`; full pass `sol_906e878698734d6c` |

Jev's POST gates were open, weakest confidence 0.97 for resource routing and 0.96 for scope routing. Confidence describes interpretation, not correctness. The generic slice check ran without solver cache; family receipts disclose cached identical normalized results where applicable. These checks verify explicit models, not arbitrary Rust execution. The 24-row fixture is an experimental budget, not a new production constant. Actual code paths and runtime tests connect the models to this implementation.

The notebook remains at `/Volumes/Projects/spur/spur-performance-rust-poc-20261009.ipynb`. Its original RED/GREEN Rust cells and provenance check were rerun after freezing the original baseline sources. Frozen-source artifacts are committed as `52f18080` in `/Volumes/Projects/spur-notebook`, so the original comparison remains reproducible after this patch.

The earlier POC measured 7.154 ms to 0.112 ms for a 5,000-row warm renderer fixture and 21.068 ms to 9.294 ms for a 5,000-ID SQLite fixture. Those are microbenchmarks with adapters, not measurements of the complete patched application. PID 94810 was the original profile target; the current conversation process was excluded. No modified binary was started or attached to that PID. End-to-end CPU/frame/snapshot latency and the original 5.29 GiB footprint still require a representative rebuilt-process profile.

Independent read-only review found no blocking TUI defects and confirmed both filtered-snapshot findings were resolved. Optional additional coverage for styled multi-span rows and width zero was identified; styled output is already covered by the notebook comparison.
