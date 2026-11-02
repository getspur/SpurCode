# Plan inspector low-friction follow-up

Tracking epic: `bd-kep`. Approved scope: six inline UX improvements, each with a persisted PRE solve, observed behavioral RED, separate test commit, implementation, remote GREEN, and persisted POST solve. Production changes stay within `spur-tui`; the old inspector plan and obsolete I3 branches were left alone.

Design: [approved follow-up spec](../../specs/2026-11-02-plan-inspector-low-friction-design.md). Execution: [implementation plan](../../plans/2026-11-02-plan-inspector-low-friction.md). Controls: [user documentation](../../../user-docs/04-issues-and-planning.md).

## Delivered changes and traceability

Each evidence file contains the exact solver requests/results, failing assertions, remote verification commands, and scope of the proof.

| Task | Result | Test commit | Fix commit | Evidence |
| --- | --- | --- | --- | --- |
| U1 / `bd-3od` | Command failures remain visible in the compact header and complete text remains available in help. | `6036f1d39` | `75b9d7c9b` | [PRE, RED, GREEN, POST](U1.md) |
| U2 / `bd-g9b` | Linked issues open at their own content; closing restores the task's viewport and pending-bottom intent. | `a19530c16` | `16b810d5f` | [PRE, RED, GREEN, POST](U2.md) |
| U3 / `bd-3mr` | Compact footers retain Esc close, help, scrolling and pane-switch controls at 40 columns. | `388dc1643` | `26d2e0f68` | [PRE, RED, GREEN, POST](U3.md) |
| U4 / `bd-2qa` | Stacked rows budget long titles around priority metadata, preserve graphemes and show elision. | `80fa1134c` | `7e8d89871` | [PRE, RED, GREEN, POST](U4.md) |
| U5 / `bd-1aj` | Eight essential inspector text tokens use stronger contrast in the three resolved built-in themes. | `ed4e1e866` | `5c217e027` | [PRE, RED, GREEN, POST](U5.md) |
| U6 / `bd-34h` | `/` finds tasks by name/task ID/issue ID; `[` and `]` cycle tasks needing attention. | `a18a332e4` | `11ff6da1a` | [PRE, RED, GREEN, POST](U6.md) |
| U6R1 / `bd-8ug` | Actual App dispatch preserves bare `u` and literal Unicode in finder/retry input, while retaining global palette/quit ownership. | `036b8e27d` | `ed735db97` | [PRE, RED, GREEN, POST](R1.md) |
| U6R2 / `bd-2c3` | Actual App navigation feedback appears in the header, including alongside a persistent command error. | `442efeb31` | `434a9b2c6` | [PRE, RED, GREEN, POST](R2.md) |

Finder cancellation preserves the selected task, focus and scroll position. Search selection remains keyed by task ID across snapshots. Attention navigation skips approved, cancelled and superseded history. Tests cover modal ownership, Unicode text and deletion, compact frames, long wrapped output, and issue/task viewport restoration.

## Final verification

Verified source: `434a9b2c6`, relative to base `05ec7118e`. The full remote crate suite exited 0: **2,151 passed, 0 failed, 4 ignored**, across 118 result blocks including doc-tests. Production-library clippy also exited 0 with warnings treated as errors.

```bash
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-tui
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo clippy -p spur-tui --lib --no-deps -- -D warnings
```

Full-suite log: `/private/tmp/plan-ux-final-tests-after-review.log`. The wrapper completed and released remote queue ticket 32. No local fallback ran. An earlier infrastructure-only attempt, before the review follow-ups, exited 200 without executing tests; it is not counted as behavioral RED or final verification.

Clippy log: `/private/tmp/plan-ux-final-clippy.log`. The wrapper completed and released queue ticket 33. This lint run covers the production library with `--no-deps`; the full test command above covers the library and integration suites.

Scoped `rustfmt --edition 2021 --check` and whitespace checks pass. Unrelated workspace edits remain outside the commits for this task. The full test command reports existing warnings for `spur-graph::FileOidMatch::as_bool` and an unnecessary `mut` in `tests/command_registry.rs`; neither is introduced by these changes.

## Independent review

The first independent review of `05ec7118e..11ff6da1a` found three Important issues at the App integration boundary: bare-u interception, literal-Unicode navigation normalization, and invisible transient hints. Each was reproduced through actual App dispatch before the follow-up fixes above.

Follow-up review of `11ff6da1a..434a9b2c6` found no remaining Critical or Important issues. The reviewer made no production edits. Final full-suite and lint verification completed after that review, with no further production changes.

## Solver and measurement scope

Geometry POST checks use measured terminal-cell extents from rendered TestBackend screens where reported; derived row budgets are identified explicitly in each task record. Essential footer controls have a 40-column witness. Tiny terminals can only display the space they provide.

Workflow checks cover the declared finite traces. Equivalent cached POST solver envelopes are retained and labeled in U2, U6 and R1; they are repeated verification requests, not independent solver executions. Runtime regressions establish the mapping from implementation behavior to those traces.

Contrast POST checks cover 48 resolved token/background pairs: eight changed text tokens, three built-in YAML themes and two declared surfaces. The measured minimum is 4.6646:1. Palette constructors and resolved YAML themes differ; U5 records the correction to the earlier constructor-only light-theme estimate. Custom overrides and other accent tokens are outside that result.

These checks establish the documented geometry, contrast and interaction behavior. They do not measure user-study outcomes or prove unbounded workflow liveness.
