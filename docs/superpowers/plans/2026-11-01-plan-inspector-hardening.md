# Plan Inspector Hardening Implementation Plan

> **For SPUR orchestrator:** This plan is designed for submit_plan with existing beads issues.
> Every task is a child of bd-2mr and has explicit dependencies and file scope.

**Source spec:** `docs/superpowers/specs/2026-11-01-plan-inspector-hardening-design.md`
**Formal @spec cells:** none; Markdown specification
**Design authorization:** User's 2026-11-01 request to address every reviewed issue with TDD and pre/post solve.
**Implementation epic:** `bd-2mr`
**Goal:** Correct and usable plan inspection across long plans, long output, and all task states.
**Architecture:** Preserve stage boards with a readable stage viewport and selection visibility. Add independently scrollable detail focus, complete status classification, truthful summaries, and mode-aware help.
**Tech stack:** Rust 2021, ratatui TestBackend, existing unicode-width/unicode-segmentation, SPUR solver MCP.

## Routing and dependency DAG

I1 (board window) -> I2 (selected rows) -> I4 (status completeness)
I3 (detail scrolling) -----------------> I4
I4 -> I5 -> I6 -> I7 -> I8 -> I9 -> I10

I1 and I3 have disjoint production files and can run independently. Their public renderer interfaces remain compatible. Later tasks serialize because they share inspector/detail files. Codex handles I1/I2's focused component changes; claude-code handles coordinated renderer/view tests. No worker model override is requested.

## Required cycle for every task

1. Navigate solve_rule_spec; execute applicable implemented catalog rules. Only use generic constraints after proving a catalog miss and loading solve_constraint_spec plus preflight solve_constraint_check.
2. Persist SOLVE PRE and record exact encoded facts, assumptions, status interpretation, and solve_id.
3. Add the named behavior regressions below using real renderers/Views and TestBackend. Run and observe the expected assertion failure. Commit `test(spur-tui): I<n> ...`.
4. Implement the stated contract inside the listed files. Run focused new tests and relevant existing inspector tests; verify zero failures.
5. Persist SOLVE POST using actual landed geometry, constants, status facts, or transition policy. Do not claim universal coverage beyond the encoded domain.
6. Record evidence in the per-task review file and beads audit. Commit `fix(spur-tui): I<n> ...`.
7. Brain verifies issue/audit/signals, complete diff, RED/GREEN output, and replayable pre/post evidence before approval.

Test commands use `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-tui ...`. A failing compiler or unavailable network is not RED. Local fmt uses scripts/spur-cargo. Preserve modal ownership and unrelated files.

## Task I1: Window stage lanes to preserve readable task labels

**Issue:** `bd-1o3`
**Task ID:** `I1`
**Depends on:** none
**Suggested worker:** `codex`
**Files:**
- `crates/spur-tui/src/components/plan_stage_board.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I1.md`

**Acceptance contract:** At terminal widths 90, 120, and 200 with 18 stages, render a contiguous stage window containing the selected stage. Reserve at least ten ASCII title cells plus the existing four marker/status cells and two border cells whenever the board extent permits it; handle smaller extents without panic. Derive lane capacity from catalog-backed cell geometry, expose hidden-stage/window position, and elide long labels visibly. Keep render_stage_board's public call signature compatible so the independent scrolling task can run concurrently.

**SOLVE PRE:** Use design layout.axis_capacity for minimum lane capacity and design layout.containment/non_overlap for the declared visible stage rectangles. PRE establish a feasible window plus an overflowing all-stage counterexample; POST use implemented constants and actual geometry emitted by the renderer/helper.

**RED test behavior:** Add TestBackend regressions in the existing board test module: 18 stages, each with a distinct ten-cell label; select the last stage in a 120x20 board and assert the selected label and full Stage 17 header are readable, along with an overflow/window indication. Exercise selection at both ends and board extents below the minimum. The current equal-ratio implementation must fail the readable-last-stage assertion.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I1.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I2: Keep selected tasks visible in both board layouts

**Issue:** `bd-2e6`
**Task ID:** `I2`
**Depends on:** `I1`
**Suggested worker:** `codex`
**Files:**
- `crates/spur-tui/src/components/plan_stage_board.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I2.md`

**Acceptance contract:** Keep the entire selected task row visible in a crowded wide lane and a long stage-grouped board. Account for stage headings, separators, metadata lines, borders, and viewport height. Recompute after resize and blocker jumps. Avoid a global board scroll state that can desynchronize selection.

**SOLVE PRE:** Use catalog design containment for selected rendered row inside the board inner viewport; for an uncatalogued discrete scroll-offset policy navigate generic spec/check before asserting a bounded counterexample. PRE and POST must cover selected row/height/viewport residual bounds and report the exact encoded domain.

**RED test behavior:** Use the existing board fixtures and actual renderers. Create twenty tasks in one stage and select the last in a short viewport; assert its selection marker/title is visible. Repeat for twenty one-task stages in stacked mode, including selected first/middle/last and shrinking height. The current unscrolled Paragraph must fail.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I2.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I3: Make task detail independently scrollable and clamp wrapped rows

**Issue:** `bd-3kb`
**Task ID:** `I3`
**Depends on:** none
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_task_detail.rs`
- `crates/spur-tui/tests/plan_inspector_scrolling.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I3.md`

**Acceptance contract:** Allow focus/scroll of task output without opening an issue, including tasks with no issue ID. Add Tab focus between board and detail, preserve existing issue-open j/k scrolling, use actual wrapped visual row counts, clamp persisted scroll offsets, and ensure G then k/PageUp moves away from bottom immediately. Keep issue/peek/confirmation modal ownership intact. Keep existing render_task_detail signature compatible where possible; any required signature update must update its inspector call sites within this task.

**SOLVE PRE:** PRE catalog design containment/axis capacity over wrapped row facts, plus validated generic bounded scroll transition model only where no catalog rule captures clamp and bottom-to-up. POST re-encode the landed clamp/wrapping policy and verify no selected scroll state exceeds its rendered maximum.

**RED test behavior:** Write actual View/TestBackend regressions for a long wrapped summary with no linked issue, issue-open wrapped body with a visible tail marker, G followed by k and PageUp, repeated down beyond end then up, and focus/modal isolation. Also preserve existing plan_inspector_snapshot issue scrolling tests. Verify expected behavioral RED before implementation.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I3.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I4: Include escalations in progress and task status presentation

**Issue:** `bd-2vm`
**Task ID:** `I4`
**Depends on:** `I2`, `I3`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_stage_board.rs`
- `crates/spur-tui/tests/plan_inspector_statuses.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I4.md`

**Acceptance contract:** Include counts.escalated in the task denominator and show an explicit attention/escalation count and glyph/style for escalated_to_brain. One approved plus one escalated must render 1 / 2 approved/done and never a full success gauge. Escalation-only plans must not render 0 / 0. Avoid changing ACP/core contracts.

**SOLVE PRE:** Use catalog data_integrity aggregate_balance/mutually_consistent or applicable discovered rules to bind status rows to complete totals; finite status coverage can use catalog workflow safety. PRE show current missing-escalation counterexample and desired feasible model; POST verify observed implemented snapshots and all finite status cases.

**RED test behavior:** Render real PlanSnapshotUpdated fixtures containing one approved plus one escalated and escalation-only plans. Assert complete totals, explicit escalation label, a defined non-question-mark task glyph and attention state. Add a task-status table coverage test including all currently emitted backend status strings.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I4.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I5: Show setup conflicts as explicit blocked work

**Issue:** `bd-3ee`
**Task ID:** `I5`
**Depends on:** `I4`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_task_detail.rs`
- `crates/spur-tui/tests/plan_inspector_statuses.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I5.md`

**Acceptance contract:** Treat blocked_on_setup_conflict as an explicit blocked condition in header attention counts and detail banner even with an empty blocked_by list. Keep structural dependency waiting distinct from setup conflicts. Include conflict reason/file details in the selected pane, without falsely classifying all pending tasks as conflicts.

**SOLVE PRE:** Navigate catalog workflow safety or data_integrity conditional_required/mutually_consistent for the finite classification table. PRE encode setup-conflict/no-dependencies witness; POST verify rendered classification facts for pending, blocked dependency, and setup conflict.

**RED test behavior:** Add a setup-conflict snapshot with no blocked_by entries and assert an explicit blocked/setup-conflict header label and visible BLOCKED banner/reason. Add ordinary pending without blockers as a negative case and pending with blockers as a dependency-wait case. Reuse I4 status fixtures without weakening tests.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I5.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I6: Make worker labels and overlapping task counts truthful

**Issue:** `bd-36e`
**Task ID:** `I6`
**Depends on:** `I5`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_task_detail.rs`
- `crates/spur-tui/tests/plan_inspector_statuses.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I6.md`

**Acceptance contract:** Render terminal executor nodes as last worker/succeeded/failed/cancelled without the word live; reserve live for active phases. Present disjoint lifecycle status counts and separately labeled overlapping attention/dependency-blocked counts. Split cancelled versus superseded for the inspector using task rows where needed, since backend counts.cancelled combines them. Preserve complete progress denominator including escalations.

**SOLVE PRE:** Use catalog data_integrity aggregate_balance and mutually_consistent/conditional_required for the status/count and worker-live truth tables; use workflow safety if it fits better after discovery. PRE and POST must include active and each terminal phase, overlap classification, and complete count conservation.

**RED test behavior:** Render approved task with only a succeeded executor and assert no live succeeded claim; exercise active worker positive case. Render approved/pending-ready/cancelled/superseded/dependency-blocked mix and assert lifecycle totals are unambiguous and superseded is not mislabeled cancelled.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I6.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I7: Keep next action visible and restrict risks to actionable facts

**Issue:** `bd-2ns`
**Task ID:** `I7`
**Depends on:** `I6`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I7.md`

**Acceptance contract:** Always retain an intelligible next action alongside actionable risk summary. Deduplicate failed blockers referenced by multiple pending tasks. Exclude historical retry chips from approved/cancelled/superseded tasks in operator risk summary. Do not present plan.updated_at as a task failure timestamp; omit unsupported age or label it honestly. Preserve running-worker elapsed information when evidence is available.

**SOLVE PRE:** Catalog data_integrity unique plus conditional_required/mutually_consistent or workflow safety over finite risk facts. PRE encode duplicate/historical-retry/unknown-time witnesses; POST verify implemented summary facts and bounded display capacity when fitting next+risk text.

**RED test behavior:** Add existing-module tests for an approved task at attempt 2/3 with next_action, two pending tasks sharing one failed blocker, and a recently refreshed plan with an old/unknown failure timestamp. Assert one blocker entry, no terminal retry risk, next action remains visible, and no invented failure age. Include narrow-render assertion of next action identity.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I7.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I8: Protect header state and choose layout from usable geometry

**Issue:** `bd-mig`
**Task ID:** `I8`
**Depends on:** `I7`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I8.md`

**Acceptance contract:** Shorten displayed plan UUID while preserving a way to inspect full ID in an existing detail/help surface. Prioritize lifecycle state, complete progress, attention counts, and next action over source/owner identifiers. Derive layout choice from usable board/detail widths and height, with readable windows from I1; avoid unreadability at 89/90 columns. Keep header useful in short terminals and no text overlap.

**SOLVE PRE:** Use catalog design axis_capacity/containment/non_overlap over declared header, board, detail, footer rectangles. PRE synthesize feasible sizes from priority content requirements; POST supply actual emitted geometry and implemented residual bounds. Any new breakpoint/size constant must be solver-grounded before RED.

**RED test behavior:** Render UUID-based plans at 80x24, 89x24, 90x24, 120x32, 200x50 and a short-height case. Assert plan status and progress are visible, next action survives, full ID remains inspectable, and selected task stays visible across layout changes. Use real TestBackend coordinates/cell width, not mere no-panic assertions.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I8.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I9: Prioritize selected task action and expose dependency/output detail

**Issue:** `bd-1aa`
**Task ID:** `I9`
**Depends on:** `I8`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_task_detail.rs`
- `crates/spur-tui/tests/plan_inspector_details.rs`
- `docs/superpowers/reviews/plan-inspector-hardening/I9.md`

**Acceptance contract:** Make concise task identity and status primary; surface errors, actionable next step and blocker reason before verbose summary/branch/delegation metadata. Preserve multiline output. Add keyboard-accessible full output/metadata and navigable parent/child relationships using existing task IDs, with an explicit local dependency focus or compact expansion. Keep b blocker behavior, issue loading, independent detail scrolling and confirmation/peek ownership.

**SOLVE PRE:** Catalog design containment/capacity for actionable content plus workflow bounded_reachability/safety for local dependency navigation. PRE derive feasible layout and finite transition witness; POST verify implemented rectangles/transition table, including external missing dependency feedback and empty relationships.

**RED test behavior:** Use a long issue title/context, long multiline summary and branch, plus an error and next action; assert error/action are visible at scroll zero and all output remains reachable. Create multiple parents/children exceeding compact dependency columns; assert full IDs can be expanded and a chosen parent/child is selectable with keyboard while blocker cycling remains correct.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I9.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.

## Task I10: Make inspector help mode-correct and text cell-width aware

**Issue:** `bd-ptq`
**Task ID:** `I10`
**Depends on:** `I9`
**Suggested worker:** `claude-code`
**Files:**
- `crates/spur-tui/src/views/plan_inspector.rs`
- `crates/spur-tui/src/components/plan_task_detail.rs`
- `crates/spur-tui/src/components/plan_stage_board.rs`
- `docs/user-docs/04-issues-and-planning.md`
- `docs/superpowers/reviews/plan-inspector-hardening/I10.md`

**Acceptance contract:** Show footer commands for the active focus/mode only; issue-open/detail-focused j/k must say scroll, board says task, modal/peek ownership remains exclusive. Add ? help/legend including every emitted task glyph, focus, stage windows, full plan identity, dependency navigation and return keys; help must remain accessible at narrow widths. Correct inspector truncation/padding/footer fitting to terminal display cells and grapheme boundaries using existing unicode-width/unicode-segmentation dependencies. Update docs to actual controls/glyphs and dependency-depth stages, avoiding unsupported global stage-barrier claims.

**SOLVE PRE:** PRE catalog design axis_capacity/containment over measured display-cell extents and workflow safety for command ownership; generic bounded budgets only if no catalog rule covers a required property. POST remeasure landed strings/geometry across the finite modes and widths and verify exclusive ownership. Run full spur-tui tests, local fmt check, and remote targeted clippy for integrated final verification.

**RED test behavior:** Add mode-by-width footer assertions for browse, detail, issue, peek and confirmations; help open/close and complete status legend; Unicode task titles/dependencies/status messages containing CJK, combining marks and emoji; zero/one/small width truncation budgets; assert rendered cell width never exceeds allocations and ellipsis consumes its budget.

**GREEN implementation:** Change the existing renderer/navigation/classifier responsible for the failed behavior; preserve its public callers outside the listed scope. The complete contract above defines the resulting behavior. Add no unrelated refactors.

**SOLVE POST:** Replay the same relevant rule bindings with the actual implemented facts; verify all new witnesses and residual bounds. Record exact results and avoid treating inconclusive statuses as passing.

**Evidence and commits:** `docs/superpowers/reviews/plan-inspector-hardening/I10.md`, test commit followed by fix commit, exact commands/output/exit codes, persisted pre/post solve IDs. Only this task's listed files are writable; scope expansion requires a signal.


## Concrete initial RED example

The I1 regression can use existing plan_stage_board test helpers without inventing a production helper:

```rust
#[test]
fn selected_last_stage_keeps_its_label_readable() {
    let tasks = (0..18).map(|idx| {
        let mut row = task(&format!("stage-{idx:04}"), "ready");
        row.stage_idx = idx;
        row
    }).collect();
    let plan = plan_with_tasks(tasks);
    let lineage = ExecutorLineage::new();
    let theme = load_built_in("dark").unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal.draw(|frame| {
        render_stage_board(frame, frame.area(), &plan, "stage-0017", 17, &lineage, &theme);
    }).unwrap();
    let rendered = rendered_buffer_text(&terminal);
    assert!(rendered.contains("stage-0017"), "{rendered}");
    assert!(rendered.contains("Stage 17"), "{rendered}");
}
```

The solver PRE must run first and justify the ten-cell readability witness and derived minimum capacity. Other named regressions above describe their complete triggering state and observable outcomes; workers implement them using their listed file's existing fixture types.

## Integration gate

After all ten reviewed tasks are approved, integrate their finished commits in dependency order without overwriting unrelated working-tree changes. Run local scoped formatting verification, all inspector integration suites, the spur-tui unit/integration suite, and targeted remote clippy. Inspect the final diff against every accepted contract, including 80x24, 90x24, 120x32, and 200x50 layouts. Verify ten pre/post pairs and ten observed TDD cycles before closing bd-2mr.

