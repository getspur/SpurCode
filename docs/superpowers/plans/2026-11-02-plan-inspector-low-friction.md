# Plan inspector low-friction implementation plan

> Execution: inline, explicitly requested by the user. Do not submit this plan
> to the detached dispatcher. Each task has a new Beads issue under `bd-kep`.

**Source spec:** `docs/superpowers/specs/2026-11-02-plan-inspector-low-friction-design.md`
**Goal:** Remove the six approved sources of inspector friction.
**Architecture:** Keep view state and keyboard ownership in PlanInspectorView;
keep cell fitting and content rendering in the existing components. Extract the
task finder into a small view-local module if needed.
**Tech stack:** Rust, ratatui TestBackend, scripts/spur-cargo, catalog solver.

All tasks execute sequentially because they share the view and test fixture.
Each starts with PRE solve before its new failing test, then RED/test commit,
minimal production change, GREEN, POST solve and fix/feature commit. Exact
commands, requests and receipts go under
`docs/superpowers/reviews/plan-inspector-low-friction/`.

## U1 / bd-3od: visible action errors

Files: view, `tests/plan_inspector_hardening.rs`, U1 evidence.
Test `command_errors_remain_visible_at_compact_sizes` sends a real
PlanCommandError and a subsequent loop event, renders 40x8/80x8, and requires an
error header, retained selection and full help text. Preserve existing error
clearing tests. PRE/POST: error-row containment and header/body/footer capacity.

## U2 / bd-g9b: issue content and return position

Files: view, task-detail component, inspector scrolling/snapshot tests, U2 evidence.
Tests open an issue over long task output and require visible loading/error or
issue title/body, then close and require the prior task viewport. Cover pending
G, resize, Unicode and existing modal ownership. PRE/POST: issue first-screen
capacity and saved-position workflow witnesses.

## U3 / bd-3mr: compact pane controls

Files: view, hardening tests, U3 evidence.
Test board/detail/issue footers at 40/80 columns and 8/24 rows with event status;
require the active navigation meaning, escape/help and pane return control.
PRE/POST: measured footer fragment capacity and containment.

## U4 / bd-2qa: bounded stacked titles

Files: stage-board component, board tests, U4 evidence.
Test long ASCII and Unicode titles with retry/blocker chips at compact widths;
require ellipsis, priority metadata, selected-row visibility and bounded cells.
PRE/POST: prefix/title/gap/chip capacity including borders.

## U5 / bd-1aj: essential text contrast

Files: theme tokens, theme/inspector tests, U5 evidence.
Test essential inspector token colors under dark/light/high-contrast themes
against their declared backgrounds, and ensure decorative borders remain subtle.
PRE/POST: a11y.text_contrast with normalized sRGB luminance for built-in pairs.

## U6 / bd-34h: task finder and attention cycle

Files: view, optional view-local search module, hardening tests, user docs, U6 evidence.
Test `/`, Unicode input/backspace, result selection, Esc cancellation, empty
results, snapshot stability and modal key isolation. Test bracket navigation
through attention tasks with wraparound and explicit empty feedback.
PRE/POST: finite workflow transition/safety/reachability models plus observed
finder geometry. Existing stage and relative navigation remain covered.

## Final integration

Run the inspector integration targets and relevant library tests, then the full
remote spur-tui suite and production-library clippy. Check scoped local fmt and
whitespace. Request a read-only code review under the requesting-code-review
skill; resolve all important findings with regression-first fixes. Record final
verification and close the new issues only after review.
