# Plan Inspector Hardening Design

The user approved addressing all ten findings from the plan inspector review on 2026-11-01 and required TDD with an executed pre-solve and post-solve for every task. This document records that accepted scope and makes the behavior concrete. Implementation epic: `bd-tje`.

## User outcomes

An operator can identify the selected task, see why work needs attention, navigate the plan without losing selection, and read all output at supported terminal sizes. The inspector reports every backend task state and distinguishes active work from completed worker history. Layout remains usable for the 18-stage shape shown in the supplied screenshot.

## Interaction and geometry decisions

Keep stage lanes for wide displays, but show a contiguous readable window rather than squeezing all stages. The selected stage must always be in the window. The readability requirement is ten ASCII title cells when space permits, plus the current four marker/status cells and two borders; the solver derives capacity from actual available extent. This is a minimum readability contract, not a universal best width. Show window position and hidden-stage direction.

Keep a stage-grouped list at constrained sizes. In both modes, selected task rows remain inside the visible inner board after navigation, resize, and blocker jumps.

Add Tab focus between board and detail. Detail focus scrolls task output even without a linked issue. Existing issue-open j/k scrolling remains supported. Confirmation dialogs and stream peek retain keyboard ownership. G selects the actual last wrapped row position; moving up immediately after G must work. Wrapped visual rows, not source line entries, determine scroll limits.

The selected task pane prioritizes concise identity, task status, error/blocker reason, and actionable next step. Long output preserves line breaks and remains fully reachable. Verbose execution metadata and issue context are accessible by progressive disclosure. Parent and child relationships can be expanded and navigated by task ID; b continues to cycle actual blockers.

Use a mode-specific footer and ? help/legend. Full plan identity remains inspectable after shortening the displayed header ID. All truncation, padding, and width budgets use terminal cells and preserve grapheme boundaries.

## Status and count decisions

All emitted task states have an explicit glyph and meaningful label, including escalated_to_brain. Complete progress includes escalated tasks. The numerator is labeled approved when it counts approved tasks.

Display disjoint lifecycle counts separately from overlapping dependency/attention counts. Derive cancelled and superseded inspector labels from task rows where the legacy aggregate combines them. Setup conflicts are explicitly blocked even though their blocked_by list is empty and their aggregate is pending.

Only active executor phases are labeled live. Terminal nodes are worker history. The next action remains visible alongside deduplicated actionable risk facts. Completed retry history does not become a current risk. Do not infer failure age from plan.updated_at.

## Accepted task contracts

### I1: Window stage lanes to preserve readable task labels

Issue: `bd-3a4`. At terminal widths 90, 120, and 200 with 18 stages, render a contiguous stage window containing the selected stage. Reserve at least ten ASCII title cells plus the existing four marker/status cells and two border cells whenever the board extent permits it; handle smaller extents without panic. Derive lane capacity from catalog-backed cell geometry, expose hidden-stage/window position, and elide long labels visibly. Keep render_stage_board's public call signature compatible so the independent scrolling task can run concurrently.

Regression witnesses: Add TestBackend regressions in the existing board test module: 18 stages, each with a distinct ten-cell label; select the last stage in a 120x20 board and assert the selected label and full Stage 17 header are readable, along with an overflow/window indication. Exercise selection at both ends and board extents below the minimum. The current equal-ratio implementation must fail the readable-last-stage assertion.

Solver contract: Use design layout.axis_capacity for minimum lane capacity and design layout.containment/non_overlap for the declared visible stage rectangles. PRE establish a feasible window plus an overflowing all-stage counterexample; POST use implemented constants and actual geometry emitted by the renderer/helper.

### I2: Keep selected tasks visible in both board layouts

Issue: `bd-32o`. Keep the entire selected task row visible in a crowded wide lane and a long stage-grouped board. Account for stage headings, separators, metadata lines, borders, and viewport height. Recompute after resize and blocker jumps. Avoid a global board scroll state that can desynchronize selection.

Regression witnesses: Use the existing board fixtures and actual renderers. Create twenty tasks in one stage and select the last in a short viewport; assert its selection marker/title is visible. Repeat for twenty one-task stages in stacked mode, including selected first/middle/last and shrinking height. The current unscrolled Paragraph must fail.

Solver contract: Use catalog design containment for selected rendered row inside the board inner viewport; for an uncatalogued discrete scroll-offset policy navigate generic spec/check before asserting a bounded counterexample. PRE and POST must cover selected row/height/viewport residual bounds and report the exact encoded domain.

### I3: Make task detail independently scrollable and clamp wrapped rows

Issue: `bd-uw9`. Allow focus/scroll of task output without opening an issue, including tasks with no issue ID. Add Tab focus between board and detail, preserve existing issue-open j/k scrolling, use actual wrapped visual row counts, clamp persisted scroll offsets, and ensure G then k/PageUp moves away from bottom immediately. Keep issue/peek/confirmation modal ownership intact. Keep existing render_task_detail signature compatible where possible; any required signature update must update its inspector call sites within this task.

Regression witnesses: Write actual View/TestBackend regressions for a long wrapped summary with no linked issue, issue-open wrapped body with a visible tail marker, G followed by k and PageUp, repeated down beyond end then up, and focus/modal isolation. Also preserve existing plan_inspector_snapshot issue scrolling tests. Verify expected behavioral RED before implementation.

Solver contract: PRE catalog design containment/axis capacity over wrapped row facts, plus validated generic bounded scroll transition model only where no catalog rule captures clamp and bottom-to-up. POST re-encode the landed clamp/wrapping policy and verify no selected scroll state exceeds its rendered maximum.

### I4: Include escalations in progress and task status presentation

Issue: `bd-1hq`. Include counts.escalated in the task denominator and show an explicit attention/escalation count and glyph/style for escalated_to_brain. One approved plus one escalated must render 1 / 2 approved/done and never a full success gauge. Escalation-only plans must not render 0 / 0. Avoid changing ACP/core contracts.

Regression witnesses: Render real PlanSnapshotUpdated fixtures containing one approved plus one escalated and escalation-only plans. Assert complete totals, explicit escalation label, a defined non-question-mark task glyph and attention state. Add a task-status table coverage test including all currently emitted backend status strings.

Solver contract: Use catalog data_integrity aggregate_balance/mutually_consistent or applicable discovered rules to bind status rows to complete totals; finite status coverage can use catalog workflow safety. PRE show current missing-escalation counterexample and desired feasible model; POST verify observed implemented snapshots and all finite status cases.

### I5: Show setup conflicts as explicit blocked work

Issue: `bd-twt`. Treat blocked_on_setup_conflict as an explicit blocked condition in header attention counts and detail banner even with an empty blocked_by list. Keep structural dependency waiting distinct from setup conflicts. Include conflict reason/file details in the selected pane, without falsely classifying all pending tasks as conflicts.

Regression witnesses: Add a setup-conflict snapshot with no blocked_by entries and assert an explicit blocked/setup-conflict header label and visible BLOCKED banner/reason. Add ordinary pending without blockers as a negative case and pending with blockers as a dependency-wait case. Reuse I4 status fixtures without weakening tests.

Solver contract: Navigate catalog workflow safety or data_integrity conditional_required/mutually_consistent for the finite classification table. PRE encode setup-conflict/no-dependencies witness; POST verify rendered classification facts for pending, blocked dependency, and setup conflict.

### I6: Make worker labels and overlapping task counts truthful

Issue: `bd-pu1`. Render terminal executor nodes as last worker/succeeded/failed/cancelled without the word live; reserve live for active phases. Present disjoint lifecycle status counts and separately labeled overlapping attention/dependency-blocked counts. Split cancelled versus superseded for the inspector using task rows where needed, since backend counts.cancelled combines them. Preserve complete progress denominator including escalations.

Regression witnesses: Render approved task with only a succeeded executor and assert no live succeeded claim; exercise active worker positive case. Render approved/pending-ready/cancelled/superseded/dependency-blocked mix and assert lifecycle totals are unambiguous and superseded is not mislabeled cancelled.

Solver contract: Use catalog data_integrity aggregate_balance and mutually_consistent/conditional_required for the status/count and worker-live truth tables; use workflow safety if it fits better after discovery. PRE and POST must include active and each terminal phase, overlap classification, and complete count conservation.

### I7: Keep next action visible and restrict risks to actionable facts

Issue: `bd-1k0`. Always retain an intelligible next action alongside actionable risk summary. Deduplicate failed blockers referenced by multiple pending tasks. Exclude historical retry chips from approved/cancelled/superseded tasks in operator risk summary. Do not present plan.updated_at as a task failure timestamp; omit unsupported age or label it honestly. Preserve running-worker elapsed information when evidence is available.

Regression witnesses: Add existing-module tests for an approved task at attempt 2/3 with next_action, two pending tasks sharing one failed blocker, and a recently refreshed plan with an old/unknown failure timestamp. Assert one blocker entry, no terminal retry risk, next action remains visible, and no invented failure age. Include narrow-render assertion of next action identity.

Solver contract: Catalog data_integrity unique plus conditional_required/mutually_consistent or workflow safety over finite risk facts. PRE encode duplicate/historical-retry/unknown-time witnesses; POST verify implemented summary facts and bounded display capacity when fitting next+risk text.

### I8: Protect header state and choose layout from usable geometry

Issue: `bd-27u`. Shorten displayed plan UUID while preserving a way to inspect full ID in an existing detail/help surface. Prioritize lifecycle state, complete progress, attention counts, and next action over source/owner identifiers. Derive layout choice from usable board/detail widths and height, with readable windows from I1; avoid unreadability at 89/90 columns. Keep header useful in short terminals and no text overlap.

Regression witnesses: Render UUID-based plans at 80x24, 89x24, 90x24, 120x32, 200x50 and a short-height case. Assert plan status and progress are visible, next action survives, full ID remains inspectable, and selected task stays visible across layout changes. Use real TestBackend coordinates/cell width, not mere no-panic assertions.

Solver contract: Use catalog design axis_capacity/containment/non_overlap over declared header, board, detail, footer rectangles. PRE synthesize feasible sizes from priority content requirements; POST supply actual emitted geometry and implemented residual bounds. Any new breakpoint/size constant must be solver-grounded before RED.

### I9: Prioritize selected task action and expose dependency/output detail

Issue: `bd-1ji`. Make concise task identity and status primary; surface errors, actionable next step and blocker reason before verbose summary/branch/delegation metadata. Preserve multiline output. Add keyboard-accessible full output/metadata and navigable parent/child relationships using existing task IDs, with an explicit local dependency focus or compact expansion. Keep b blocker behavior, issue loading, independent detail scrolling and confirmation/peek ownership.

Regression witnesses: Use a long issue title/context, long multiline summary and branch, plus an error and next action; assert error/action are visible at scroll zero and all output remains reachable. Create multiple parents/children exceeding compact dependency columns; assert full IDs can be expanded and a chosen parent/child is selectable with keyboard while blocker cycling remains correct.

Solver contract: Catalog design containment/capacity for actionable content plus workflow bounded_reachability/safety for local dependency navigation. PRE derive feasible layout and finite transition witness; POST verify implemented rectangles/transition table, including external missing dependency feedback and empty relationships.

### I10: Make inspector help mode-correct and text cell-width aware

Issue: `bd-2p1`. Show footer commands for the active focus/mode only; issue-open/detail-focused j/k must say scroll, board says task, modal/peek ownership remains exclusive. Add ? help/legend including every emitted task glyph, focus, stage windows, full plan identity, dependency navigation and return keys; help must remain accessible at narrow widths. Correct inspector truncation/padding/footer fitting to terminal display cells and grapheme boundaries using existing unicode-width/unicode-segmentation dependencies. Update docs to actual controls/glyphs and dependency-depth stages, avoiding unsupported global stage-barrier claims.

Regression witnesses: Add mode-by-width footer assertions for browse, detail, issue, peek and confirmations; help open/close and complete status legend; Unicode task titles/dependencies/status messages containing CJK, combining marks and emoji; zero/one/small width truncation budgets; assert rendered cell width never exceeds allocations and ellipsis consumes its budget.

Solver contract: PRE catalog design axis_capacity/containment over measured display-cell extents and workflow safety for command ownership; generic bounded budgets only if no catalog rule covers a required property. POST remeasure landed strings/geometry across the finite modes and widths and verify exclusive ownership. Run full spur-tui tests, local fmt check, and remote targeted clippy for integrated final verification.


## Delivery and verification

Every task executes: catalog discovery -> persisted SOLVE PRE -> behavioral RED observed -> test commit -> minimal implementation -> focused GREEN observed -> persisted SOLVE POST against landed policy -> fix commit -> independent review. A network/compile failure is not behavioral RED. Unknown/timeout is not a solver proof.

Use only scripts/spur-cargo for Rust commands. Build/test remotely with SPUR_REMOTE=1 and SPUR_NO_LOCAL_FALLBACK=1; use approved network escalation where sandbox network restrictions prevent AWS access. Keep fmt local and scoped. Final integration runs the inspector integration tests, the spur-tui suite, and targeted clippy as appropriate.

Preserve unrelated working-tree edits. Scope production changes to spur-tui; inspector user documentation is included. Record evidence in docs/superpowers/reviews/plan-inspector-hardening/I*.md and beads.

