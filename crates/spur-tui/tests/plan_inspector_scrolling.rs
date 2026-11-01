use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use spur_acp::{
    PlanSnapshot, PlanSnapshotCounts, PlanSnapshotTask, SessionId, SpurEvent, SpurEventBody,
};
use spur_core::{ExecutorLineage, PlanProjectionStore, SessionSynopsisProjection};
use spur_tui::{
    action::Action,
    app::BrainStatus,
    views::{plan_inspector::PlanInspectorView, View, ViewContext},
};

fn projection(issue: bool) -> PlanProjectionStore {
    projection_with_summary(
        issue,
        format!("{} OUTPUT_TAIL", "wrapped-output ".repeat(300)),
    )
}
fn projection_with_summary(issue: bool, summary: String) -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("scroll-brain".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "scroll-plan".into(),
            epic_id: None,
            status: "running".into(),
            progress: "1/1 reviewed".into(),
            next_action: "inspect output".into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                approved: 1,
                ..Default::default()
            },
            tasks: vec![PlanSnapshotTask {
                task_id: "scroll-task".into(),
                task_name: "scroll-task".into(),
                agent: "codex".into(),
                issue_id: issue.then(|| "bd-scroll".into()),
                issue_title: None,
                status: "approved".into(),
                attempt: 1,
                max_attempts: 3,
                depends_on: vec![],
                blocked_by: vec![],
                unblocks: vec![],
                summary: Some(summary),
                feedback: None,
                error: None,
                worker_branch: None,
                delegation_id: None,
                diff_summary: None,
                mutation_id: None,
                superseded_by: vec![],
                next_action: String::new(),
            }],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}
fn key(view: &mut PlanInspectorView, ctx: &ViewContext<'_>, code: KeyCode) -> Option<Action> {
    view.handle_key(KeyEvent::new(code, KeyModifiers::NONE), ctx)
}
fn render(view: &mut PlanInspectorView, ctx: &ViewContext<'_>) -> String {
    render_size(view, ctx, 100, 16)
}
fn render_size(
    view: &mut PlanInspectorView,
    ctx: &ViewContext<'_>,
    width: u16,
    height: u16,
) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), ctx))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn task_output_without_issue_has_independent_focus_and_reachable_tail() {
    let plans = projection(false);
    let lineage = ExecutorLineage::new();
    let synopsis = SessionSynopsisProjection::new();
    let ctx = ViewContext {
        lineage: &lineage,
        plan_projection: &plans,
        synopsis: &synopsis,
        brain_status: &BrainStatus::Idle,
        flag_summary: None,
        tombstone: None,
        transient_hint_override: None,
        notebook_ready: false,
        theme: spur_tui::theme::fallback_theme(),
    };
    let mut view = PlanInspectorView::new(SessionId("scroll-brain".into()));
    assert!(!render(&mut view, &ctx).contains("OUTPUT_TAIL"));
    key(&mut view, &ctx, KeyCode::Tab);
    assert!(
        matches!(
            key(&mut view, &ctx, KeyCode::Down),
            Some(Action::ScrollDown)
        ),
        "Tab must focus task output without requiring a linked issue"
    );
    key(&mut view, &ctx, KeyCode::Char('G'));
    let bottom = render(&mut view, &ctx);
    assert!(
        bottom.contains("OUTPUT_TAIL"),
        "wrapped output tail unreachable:\n{bottom}"
    );
    key(&mut view, &ctx, KeyCode::Char('k'));
    let up = render(&mut view, &ctx);
    assert_ne!(
        bottom, up,
        "one up after G must leave the bottom immediately"
    );
    key(&mut view, &ctx, KeyCode::PageUp);
    assert_ne!(
        up,
        render(&mut view, &ctx),
        "page up must move from a clamped position"
    );
    key(&mut view, &ctx, KeyCode::Char('g'));
    assert!(!render(&mut view, &ctx).contains("OUTPUT_TAIL"));
    for _ in 0..300 {
        key(&mut view, &ctx, KeyCode::Down);
    }
    let bottom = render(&mut view, &ctx);
    assert!(bottom.contains("OUTPUT_TAIL"));
    key(&mut view, &ctx, KeyCode::Up);
    assert_ne!(
        bottom,
        render(&mut view, &ctx),
        "over-scrolling must not accumulate hidden offset"
    );
    key(&mut view, &ctx, KeyCode::Tab);
    assert!(
        key(&mut view, &ctx, KeyCode::Down).is_none(),
        "Tab must return to board navigation"
    );
}

#[test]
fn issue_open_scrolls_all_wrapped_output_and_up_after_bottom() {
    let plans = projection(true);
    let lineage = ExecutorLineage::new();
    let synopsis = SessionSynopsisProjection::new();
    let ctx = ViewContext {
        lineage: &lineage,
        plan_projection: &plans,
        synopsis: &synopsis,
        brain_status: &BrainStatus::Idle,
        flag_summary: None,
        tombstone: None,
        transient_hint_override: None,
        notebook_ready: false,
        theme: spur_tui::theme::fallback_theme(),
    };
    let mut view = PlanInspectorView::new(SessionId("scroll-brain".into()));
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Enter);
    key(&mut view, &ctx, KeyCode::Char('G'));
    let bottom = render(&mut view, &ctx);
    assert!(
        bottom.contains("Loading issue detail"),
        "issue-open tail hidden by logical-line clamp:\n{bottom}"
    );
    key(&mut view, &ctx, KeyCode::Up);
    assert_ne!(
        bottom,
        render(&mut view, &ctx),
        "issue-open up after G must work"
    );
}

#[test]
fn confirmation_owns_tab_and_does_not_change_board_focus() {
    let plans = projection(false);
    let lineage = ExecutorLineage::new();
    let synopsis = SessionSynopsisProjection::new();
    let ctx = ViewContext {
        lineage: &lineage,
        plan_projection: &plans,
        synopsis: &synopsis,
        brain_status: &BrainStatus::Idle,
        flag_summary: None,
        tombstone: None,
        transient_hint_override: None,
        notebook_ready: false,
        theme: spur_tui::theme::fallback_theme(),
    };
    let mut view = PlanInspectorView::new(SessionId("scroll-brain".into()));
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Char('r'));
    key(&mut view, &ctx, KeyCode::Tab);
    key(&mut view, &ctx, KeyCode::Esc);
    assert!(
        key(&mut view, &ctx, KeyCode::Down).is_none(),
        "confirmation Tab leaked to detail focus"
    );
}

#[test]
fn large_output_bottom_is_reachable_beyond_u16_visual_rows() {
    let plans = projection_with_summary(true, String::new());
    let lineage = ExecutorLineage::new();
    let synopsis = SessionSynopsisProjection::new();
    let ctx = ViewContext {
        lineage: &lineage,
        plan_projection: &plans,
        synopsis: &synopsis,
        brain_status: &BrainStatus::Idle,
        flag_summary: None,
        tombstone: None,
        transient_hint_override: None,
        notebook_ready: false,
        theme: spur_tui::theme::fallback_theme(),
    };
    let mut view = PlanInspectorView::new(SessionId("scroll-brain".into()));
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Enter);
    let now = chrono::Utc::now();
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueDetailFetched {
            requested_id: "bd-scroll".into(),
            issue: spur_acp::IssueDetailEvent {
                id: "bd-scroll".into(),
                source: "beads".into(),
                title: "loaded issue".into(),
                status: "open".into(),
                body: format!(
                    "{}HUGE_OUTPUT_TAIL",
                    "a loaded issue line that wraps across this narrow detail pane\n"
                        .repeat(35_000)
                ),
                labels: vec![],
                assignee: None,
                url: String::new(),
                priority: None,
                issue_type: None,
                blocked_by: vec![],
                comments: vec![],
                due_at: None,
                created_at: now,
                updated_at: now,
            },
        }),
        &ctx,
    );
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Char('G'));
    let bottom = render(&mut view, &ctx);
    assert!(
        bottom.contains("HUGE_OUTPUT_TAIL"),
        "large loaded wrapped output tail is unreachable"
    );
    let wide = render_size(&mut view, &ctx, 200, 24);
    assert!(
        wide.contains("HUGE_OUTPUT_TAIL"),
        "resize must re-clamp the stored offset"
    );
    key(&mut view, &ctx, KeyCode::Up);
    assert_ne!(
        wide,
        render_size(&mut view, &ctx, 200, 24),
        "large-output up must move immediately"
    );
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Char('G'));
    assert!(render(&mut view, &ctx).contains("HUGE_OUTPUT_TAIL"));
}
#[test]
fn opening_issue_clears_a_pending_bottom_request_before_the_next_draw() {
    let plans = projection(true);
    let lineage = ExecutorLineage::new();
    let synopsis = SessionSynopsisProjection::new();
    let ctx = ViewContext {
        lineage: &lineage,
        plan_projection: &plans,
        synopsis: &synopsis,
        brain_status: &BrainStatus::Idle,
        flag_summary: None,
        tombstone: None,
        transient_hint_override: None,
        notebook_ready: false,
        theme: spur_tui::theme::fallback_theme(),
    };
    let mut view = PlanInspectorView::new(SessionId("scroll-brain".into()));
    render(&mut view, &ctx);
    key(&mut view, &ctx, KeyCode::Tab);
    key(&mut view, &ctx, KeyCode::Char('G'));
    key(&mut view, &ctx, KeyCode::Enter);
    let text = render(&mut view, &ctx);
    assert!(
        text.contains("scroll-task · codex"),
        "issue must open at top after a queued G:\n{text}"
    );
}
