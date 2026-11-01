use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
use spur_acp::{
    IssueDetailEvent, LifecycleState, PlanLifecycleEvent, PlanLoopOriginEvent, PlanOwnerStateEvent,
    PlanSnapshot, PlanSnapshotCounts, PlanSnapshotTask, PlanSummaryEvent, Role, SessionId,
    SpurEvent, SpurEventBody,
};
use spur_core::{ExecutorLineage, PlanProjectionStore, SessionSynopsisProjection};
use spur_tui::action::{Action, IssueAction};
use spur_tui::app::BrainStatus;
use spur_tui::views::plan_inspector::PlanInspectorView;
use spur_tui::views::{View, ViewContext};
use std::time::{Duration, SystemTime};

fn sample_plan_store() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("brain-1".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "plan-1".into(),
                epic_id: None,
            status: "running".into(),
            progress: "1/4 done".into(),
            next_action:
                "Use get_task_diff to review each awaiting task, then review_task to approve or reject."
                    .into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                pending: 2,
                approved: 1,
                awaiting_review: 1,
                ..Default::default()
            },
            tasks: vec![
                task("task-contract", 0, &[], Some("bd-1"), "approved"),
                task(
                    "task-projection",
                    1,
                    &["task-contract"],
                    Some("bd-2"),
                    "awaiting_review",
                ),
                task(
                    "task-app",
                    2,
                    &["task-projection"],
                    Some("bd-3"),
                    "dispatched",
                ),
                task(
                    "task-inspector",
                    3,
                    &["task-contract", "task-app"],
                    Some("bd-4"),
                    "pending",
                ),
            ],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}

fn plan_store_with_selected_task_without_issue() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("brain-1".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "plan-1".into(),
            epic_id: None,
            status: "running".into(),
            progress: "0/1 done".into(),
            next_action: "No issue IDs in this plan; use this to validate Enter feedback.".into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                pending: 1,
                ..Default::default()
            },
            tasks: vec![task("task-no-issue", 0, &[], None, "pending")],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}

fn out_of_stage_order_plan_store() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("brain-1".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "plan-1".into(),
                epic_id: None,
            status: "running".into(),
            progress: "1/4 done".into(),
            next_action:
                "Use get_task_diff to review each awaiting task, then review_task to approve or reject."
                    .into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                pending: 2,
                approved: 1,
                awaiting_review: 1,
                ..Default::default()
            },
            tasks: vec![
                task(
                    "task-projection",
                    1,
                    &["task-contract"],
                    Some("bd-2"),
                    "awaiting_review",
                ),
                task("task-contract", 0, &[], Some("bd-1"), "approved"),
                task(
                    "task-app",
                    2,
                    &["task-projection"],
                    Some("bd-3"),
                    "dispatched",
                ),
                task(
                    "task-inspector",
                    3,
                    &["task-contract", "task-app"],
                    Some("bd-4"),
                    "pending",
                ),
            ],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}

fn task(
    task_id: &str,
    stage_idx: usize,
    depends_on: &[&str],
    issue_id: Option<&str>,
    status: &str,
) -> PlanSnapshotTask {
    let blocked_by = if stage_idx == 0 {
        Vec::new()
    } else {
        depends_on.iter().map(|dep| dep.to_string()).collect()
    };
    PlanSnapshotTask {
        task_id: task_id.into(),
        task_name: task_id.into(),
        agent: "codex".into(),
        issue_id: issue_id.map(str::to_string),
        issue_title: None,
        status: status.into(),
        attempt: if status == "dispatched" { 1 } else { 0 },
        max_attempts: 3,
        depends_on: depends_on.iter().map(|dep| dep.to_string()).collect(),
        blocked_by,
        unblocks: Vec::new(),
        summary: None,
        feedback: None,
        error: None,
        worker_branch: None,
        delegation_id: if status == "dispatched" {
            Some("del-1".into())
        } else {
            None
        },
        diff_summary: None,
        mutation_id: None,
        superseded_by: Vec::new(),
        next_action: if status == "awaiting_review" {
            "review".into()
        } else {
            "wait".into()
        },
    }
}

fn sample_lineage() -> ExecutorLineage {
    use spur_acp::{LifecycleState, Role};

    let mut lineage = ExecutorLineage::new();
    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorSpawned {
        id: "exec-1".into(),
        parent_id: None,
        session_id: SessionId("worker-1".into()),
        agent: "codex".into(),
        role: Role::Executor,
        task_spec: "placeholder".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
        from: SessionId("brain-1".into()),
        to_agent: "codex".into(),
        task: "task-app".into(),
        request_id: "req-1".into(),
        delegation_plan: None,
        issue_id: Some("bd-3".into()),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
        from: SessionId("brain-1".into()),
        request_id: "req-1".into(),
        executor_id: "exec-1".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorPhaseChanged {
        id: "exec-1".into(),
        phase: LifecycleState::Running,
    }));
    lineage
}

fn lineage_with_stale_and_live_executor_for_same_issue() -> ExecutorLineage {
    use spur_acp::{LifecycleState, Role};

    let mut lineage = ExecutorLineage::new();
    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorSpawned {
        id: "exec-z".into(),
        parent_id: None,
        session_id: SessionId("worker-z".into()),
        agent: "codex".into(),
        role: Role::Executor,
        task_spec: "stale".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
        from: SessionId("brain-1".into()),
        to_agent: "codex".into(),
        task: "task-app".into(),
        request_id: "req-z".into(),
        delegation_plan: None,
        issue_id: Some("bd-3".into()),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
        from: SessionId("brain-1".into()),
        request_id: "req-z".into(),
        executor_id: "exec-z".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorPhaseChanged {
        id: "exec-z".into(),
        phase: LifecycleState::Succeeded,
    }));

    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorSpawned {
        id: "exec-a".into(),
        parent_id: None,
        session_id: SessionId("worker-a".into()),
        agent: "codex".into(),
        role: Role::Executor,
        task_spec: "live".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
        from: SessionId("brain-1".into()),
        to_agent: "codex".into(),
        task: "task-app".into(),
        request_id: "req-a".into(),
        delegation_plan: None,
        issue_id: Some("bd-3".into()),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
        from: SessionId("brain-1".into()),
        request_id: "req-a".into(),
        executor_id: "exec-a".into(),
    }));
    lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorPhaseChanged {
        id: "exec-a".into(),
        phase: LifecycleState::Running,
    }));
    lineage
}

fn plan_store_with_selected_task_app() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("brain-1".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "plan-1".into(),
                epic_id: None,
            status: "running".into(),
            progress: "1/1 done".into(),
            next_action:
                "Use get_task_diff to review each awaiting task, then review_task to approve or reject."
                    .into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                dispatched: 1,
                ..Default::default()
            },
            tasks: vec![task("task-app", 0, &[], Some("bd-3"), "dispatched")],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}

fn buffer_contains(buf: &Buffer, needle: &str) -> bool {
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s.contains(needle)
}

fn sample_issue_detail_event(id: &str, title: &str, status: &str, body: &str) -> IssueDetailEvent {
    let now = Utc::now();
    IssueDetailEvent {
        id: id.into(),
        source: "github".into(),
        title: title.into(),
        body: body.into(),
        status: status.into(),
        labels: vec!["label-a".into(), "label-b".into()],
        assignee: Some("coder".into()),
        url: "https://example.com/issue".into(),
        priority: Some(1),
        issue_type: Some("bug".into()),
        blocked_by: vec!["blocked-by-1".into()],
        comments: Vec::new(),
        due_at: None,
        created_at: now,
        updated_at: now,
    }
}

fn long_issue_body() -> String {
    (0..80)
        .map(|idx| format!("body-line-{idx:03}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn long_issue_body_with_prefix(prefix: &str) -> String {
    (0..80)
        .map(|idx| format!("{prefix}-body-line-{idx:03}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn plan_inspector_renders_wide_lane_board() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&buffer, "Stage 0"));
    assert!(buffer_contains(&buffer, "Stage 1"));
    assert!(buffer_contains(&buffer, "Task detail"));
    assert!(buffer_contains(&buffer, "codex:run"));
}

#[test]
fn plan_inspector_header_renders_loop_origin_badge() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let mut plans = sample_plan_store();
    plans.apply(&SpurEvent::now(SpurEventBody::PlansLoaded {
        plans: vec![PlanSummaryEvent {
            plan_id: "plan-1".into(),
            epic_id: "bd-plan-1".into(),
            title: "Loop-authored plan".into(),
            source_body_preview: None,
            owner_state: PlanOwnerStateEvent::Mine,
            lifecycle: PlanLifecycleEvent::Running,
            loop_origin: Some(PlanLoopOriginEvent {
                loop_id: "loopabc".into(),
                generation: 7,
            }),
            counts: None,
            updated_at: None,
            created_at: None,
        }],
        warnings: Vec::new(),
    }));
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&buffer, "⟳ gen 7"));
}

#[test]
fn plan_inspector_renders_stacked_layout_below_90_cols() {
    let backend = TestBackend::new(80, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&buffer, "Selected:"));
    assert!(buffer_contains(&buffer, "Stage 1"));
}

#[test]
fn plan_inspector_stacked_mode_j_moves_across_stage_boundaries() {
    let backend = TestBackend::new(80, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let action = view.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &ctx);
    assert!(action.is_none(), "navigation should remain in-view");
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "Selected: task-projection"),
        "stacked navigation should move to the next visible task across stages"
    );
}

#[test]
fn plan_inspector_stacked_mode_uses_visible_stage_order_when_tasks_are_permuted() {
    let backend = TestBackend::new(80, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = out_of_stage_order_plan_store();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let initial = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&initial, "Selected: task-contract"),
        "stacked mode should seed selection from the first visible task, not the raw vector order"
    );

    let action = view.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &ctx);
    assert!(action.is_none(), "navigation should remain in-view");
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let after_j = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&after_j, "Selected: task-projection"),
        "stacked mode j/k navigation should follow the rendered stage-grouped order"
    );
}

#[test]
fn plan_inspector_alt_p_requests_navigate_back() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let action = view.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT), &ctx);
    assert!(matches!(action, Some(Action::NavigateBack)));
}

#[test]
fn plan_inspector_enter_requests_issue_detail() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        action,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));
}

#[test]
fn plan_inspector_enter_no_issue_task_flashes_hint() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = plan_store_with_selected_task_without_issue();
    let lineage = sample_lineage();
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

    let action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    match action {
        Some(Action::FlashHint { message }) => {
            assert!(message.contains("No issue linked"));
        }
        _ => panic!("expected flash hint when task has no issue id"),
    }
}

#[test]
fn plan_inspector_esc_closes_open_issue_detail() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        action,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    let close_action = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(close_action.is_none());

    let back_action = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(matches!(back_action, Some(Action::NavigateBack)));
}

#[test]
fn plan_inspector_task_detail_scroll_affordance() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let open_action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        open_action,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueDetailFetched {
            requested_id: "bd-1".into(),
            issue: sample_issue_detail_event("bd-1", "Long body task", "open", &long_issue_body()),
        }),
        &ctx,
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let initial = terminal.backend().buffer().clone();
    assert!(buffer_contains(&initial, "body-line-000"));
    assert!(!buffer_contains(&initial, "body-line-020"));

    let down_action = view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(down_action, Some(Action::ScrollDown)));
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let first_page = terminal.backend().buffer().clone();
    assert_eq!(
        visible_body_indices(&first_page, "body-line-")
            .first()
            .copied(),
        visible_body_indices(&initial, "body-line-")
            .last()
            .map(|index| index + 1)
    );
    let down_action = view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(down_action, Some(Action::ScrollDown)));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let scrolled = terminal.backend().buffer().clone();
    assert_eq!(
        visible_body_indices(&scrolled, "body-line-")
            .first()
            .copied(),
        visible_body_indices(&first_page, "body-line-")
            .last()
            .map(|index| index + 1)
    );
}

#[test]
fn plan_inspector_detail_scroll_reset_when_switching_tasks() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let open_a = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        open_a,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueDetailFetched {
            requested_id: "bd-1".into(),
            issue: sample_issue_detail_event(
                "bd-1",
                "Task A",
                "open",
                &long_issue_body_with_prefix("A"),
            ),
        }),
        &ctx,
    );
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueDetailFetched {
            requested_id: "bd-2".into(),
            issue: sample_issue_detail_event(
                "bd-2",
                "Task B",
                "open",
                &long_issue_body_with_prefix("B"),
            ),
        }),
        &ctx,
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let a_top = terminal.backend().buffer().clone();
    assert!(buffer_contains(&a_top, "A-body-line-000"));
    assert!(!buffer_contains(&a_top, "A-body-line-020"));

    let down_action = view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(down_action, Some(Action::ScrollDown)));
    let down_action = view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(down_action, Some(Action::ScrollDown)));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let a_scrolled = terminal.backend().buffer().clone();
    assert!(visible_body_indices(&a_scrolled, "A-body-line-")
        .first()
        .is_some_and(|index| *index > 0));

    let lane_right = view.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &ctx);
    assert!(lane_right.is_none());

    let open_b = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(
        open_b.is_none(),
        "preloaded issue detail for task B should render immediately when opening"
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let b_open = terminal.backend().buffer().clone();
    assert!(!buffer_contains(&b_open, "A-body-line-020"));
    assert!(
        !buffer_contains(&b_open, "B-body-line-020"),
        "switching tasks should reset detail scroll away from the previous deep offset"
    );

    let down_after_switch =
        view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(down_after_switch, Some(Action::ScrollDown)));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let b_scrolled = terminal.backend().buffer().clone();
    assert_eq!(
        visible_body_indices(&b_scrolled, "B-body-line-")
            .first()
            .copied(),
        visible_body_indices(&b_open, "B-body-line-")
            .last()
            .map(|index| index + 1),
        "paging the switched task must follow its own first page"
    );
    assert!(
        !buffer_contains(&b_scrolled, "A-body-line-020"),
        "switched task detail should not inherit previous task's scroll position"
    );

    let close_b = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(close_b.is_none());

    let reopen_b = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(
        reopen_b.is_none(),
        "reopening loaded issue detail should not reissue request"
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let b_reopen = terminal.backend().buffer().clone();
    assert!(
        !buffer_contains(&b_reopen, "A-body-line-020"),
        "reopening after close should keep switched task content"
    );
    assert!(!buffer_contains(&b_reopen, "B-body-line-020"));

    let reopen_down = view.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &ctx);
    assert!(matches!(reopen_down, Some(Action::ScrollDown)));
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let b_reopen_scrolled = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&b_reopen, "B-body-line-000"),
        "reopening must reset to top before paging"
    );
    assert_eq!(
        visible_body_indices(&b_reopen_scrolled, "B-body-line-")
            .first()
            .copied(),
        visible_body_indices(&b_reopen, "B-body-line-")
            .last()
            .map(|index| index + 1)
    );

    let first_back_action = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(first_back_action.is_none());

    let second_back_action = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(matches!(second_back_action, Some(Action::NavigateBack)));
}

#[test]
fn plan_inspector_renders_issue_detail_loading_and_fetched() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        action,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let loading_buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&loading_buffer, "Loading issue detail..."));

    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueDetailFetched {
            requested_id: "bd-1".into(),
            issue: sample_issue_detail_event("bd-1", "Contract task", "open", "Detailed\nbody"),
        }),
        &ctx,
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let detail_buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&detail_buffer, "Issue"));
    assert!(buffer_contains(&detail_buffer, "id: bd-1"));
    assert!(buffer_contains(&detail_buffer, "title: Contract task"));
    assert!(buffer_contains(&detail_buffer, "status: open"));
    assert!(buffer_contains(&detail_buffer, "Description"));
    assert!(buffer_contains(&detail_buffer, "Detailed"));
}

#[test]
fn plan_inspector_issue_detail_error_without_id_targets_open_issue_when_unique() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let action = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        action,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let loading_buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&loading_buffer, "Loading issue detail..."));

    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueCommandError {
            operation: "GetIssueDetail".into(),
            error: "No issue tracker configured".into(),
            id: None,
        }),
        &ctx,
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let error_buffer = terminal.backend().buffer().clone();
    assert!(!buffer_contains(&error_buffer, "Loading issue detail..."));
    assert!(buffer_contains(&error_buffer, "Issue"));
    assert!(buffer_contains(
        &error_buffer,
        "No issue tracker configured"
    ));
}

#[test]
fn plan_inspector_issue_detail_error_without_id_ignores_with_multiple_in_flight() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let open_a = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        open_a,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-1"
    ));

    let lane_right = view.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &ctx);
    assert!(lane_right.is_none());

    let open_b = view.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &ctx);
    assert!(matches!(
        open_b,
        Some(Action::Issue(IssueAction::ViewDetail { ref id })) if id == "bd-2"
    ));

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let loading_buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&loading_buffer, "Loading issue detail..."));

    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::IssueCommandError {
            operation: "GetIssueDetail".into(),
            error: "No issue tracker configured".into(),
            id: None,
        }),
        &ctx,
    );

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let error_buffer = terminal.backend().buffer().clone();
    assert!(buffer_contains(&error_buffer, "Loading issue detail..."));
    assert!(!buffer_contains(
        &error_buffer,
        "No issue tracker configured"
    ));
}

#[test]
fn plan_inspector_esc_requests_navigate_back() {
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = sample_plan_store();
    let lineage = sample_lineage();
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

    let action = view.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &ctx);
    assert!(matches!(action, Some(Action::NavigateBack)));
}

#[test]
fn plan_inspector_prefers_live_executor_state_over_stale_higher_id() {
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = plan_store_with_selected_task_app();
    let lineage = lineage_with_stale_and_live_executor_for_same_issue();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "codex:run"),
        "board should surface the live executor rather than a stale higher-id executor"
    );
    assert!(
        buffer_contains(&buffer, "live codex running"),
        "detail pane should join against the live executor for the selected task"
    );
}

#[test]
fn plan_inspector_renders_blocked_deps_and_retry_chips() {
    let backend = TestBackend::new(160, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = plan_store_with_blocked_and_retry();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    // Navigate to Stage 1 (child task) so detail pane shows blocked_by.
    let action = view.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &ctx);
    assert!(action.is_none(), "navigation should remain in-view");
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "blocked:"),
        "board should show blocked chip for tasks with unsatisfied dependencies"
    );
    assert!(
        buffer_contains(&buffer, "↑parent"),
        "board should show upstream dependency hint"
    );
    assert!(
        buffer_contains(&buffer, "retry 2/3"),
        "board should show retry chip when attempt > 1"
    );
    assert!(
        buffer_contains(&buffer, "BLOCKED"),
        "detail pane should render a blocked banner when dependencies block the task"
    );
    assert!(
        buffer_contains(&buffer, "Blocked by"),
        "detail pane should render the blocked dependency lane"
    );
    assert!(
        buffer_contains(&buffer, "↑parent"),
        "detail pane should resolve dependency status suffixes from the current plan"
    );
}

#[test]
fn plan_inspector_renders_mixed_status_risk_banner() {
    let backend = TestBackend::new(160, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = plan_store_with_risk_mix();
    let lineage = lineage_with_long_running_ui();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "risk: lint rejected"),
        "header should surface rejected blockers"
    );
    assert!(
        buffer_contains(&buffer, "build-ui running"),
        "header should surface long-running dispatched tasks"
    );
    assert!(
        buffer_contains(&buffer, "api retry 2/3"),
        "header should surface retry pressure"
    );

    let action = view.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &ctx);
    assert!(action.is_none(), "navigation should remain in-view");
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "↑lint (REJ)"),
        "detail dependency strip should render compact resolved status suffixes"
    );
}

#[test]
fn plan_inspector_stacked_mode_shows_meta_chips() {
    let backend = TestBackend::new(80, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut view = PlanInspectorView::new(SessionId("brain-1".into()));
    let plans = plan_store_with_blocked_and_retry();
    let lineage = sample_lineage();
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

    terminal
        .draw(|frame| view.render(frame, frame.area(), &ctx))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(
        buffer_contains(&buffer, "blocked:"),
        "stacked board should show blocked chip"
    );
    assert!(
        buffer_contains(&buffer, "↑parent"),
        "stacked board should show dependency hint"
    );
    assert!(
        buffer_contains(&buffer, "retry 2/3"),
        "stacked board should show retry chip"
    );
}

fn plan_store_with_blocked_and_retry() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    store.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
        session_id: SessionId("brain-1".into()),
        snapshot: Box::new(PlanSnapshot {
            plan_id: "plan-1".into(),
            epic_id: None,
            status: "running".into(),
            progress: "0/2 done".into(),
            next_action: "waiting for dependencies".into(),
            ready_to_merge: false,
            counts: PlanSnapshotCounts {
                pending: 2,
                ..Default::default()
            },
            tasks: vec![
                PlanSnapshotTask {
                    task_id: "parent".into(),
                    task_name: "parent".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-parent".into()),
                    issue_title: None,
                    status: "approved".into(),
                    attempt: 1,
                    max_attempts: 3,
                    depends_on: vec![],
                    blocked_by: vec![],
                    unblocks: vec!["child".into()],
                    summary: None,
                    feedback: None,
                    error: None,
                    worker_branch: None,
                    delegation_id: None,
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: vec![],
                    next_action: "".into(),
                },
                PlanSnapshotTask {
                    task_id: "child".into(),
                    task_name: "child".into(),
                    agent: "kiro".into(),
                    issue_id: Some("bd-child".into()),
                    issue_title: None,
                    status: "pending".into(),
                    attempt: 2,
                    max_attempts: 3,
                    depends_on: vec!["parent".into()],
                    blocked_by: vec!["parent".into()],
                    unblocks: vec![],
                    summary: None,
                    feedback: None,
                    error: None,
                    worker_branch: None,
                    delegation_id: None,
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: vec![],
                    next_action: "wait".into(),
                },
            ],
            owner_brain_session_id: None,
            owner_token: None,
            owner_acquired_at: None,
        }),
    }));
    store
}

fn plan_store_with_risk_mix() -> PlanProjectionStore {
    let mut store = PlanProjectionStore::default();
    let occurred_at = SystemTime::now()
        .checked_sub(Duration::from_secs(18 * 60))
        .unwrap();
    store.apply(&SpurEvent {
        occurred_at,
        seq: 0,
        body: SpurEventBody::PlanSnapshotUpdated {
            session_id: SessionId("brain-1".into()),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: None,
                status: "running".into(),
                progress: "0/4 done".into(),
                next_action: "resolve risk".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    pending: 1,
                    dispatched: 1,
                    rejected: 1,
                    failed: 1,
                    ..Default::default()
                },
                tasks: vec![
                    PlanSnapshotTask {
                        task_id: "lint".into(),
                        task_name: "lint".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-lint".into()),
                        issue_title: None,
                        status: "rejected".into(),
                        attempt: 1,
                        max_attempts: 3,
                        depends_on: vec![],
                        blocked_by: vec![],
                        unblocks: vec!["release".into()],
                        summary: None,
                        feedback: Some("lint failed".into()),
                        error: None,
                        worker_branch: None,
                        delegation_id: None,
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: vec![],
                        next_action: "fix lint".into(),
                    },
                    PlanSnapshotTask {
                        task_id: "build-ui".into(),
                        task_name: "build-ui".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-build-ui".into()),
                        issue_title: None,
                        status: "dispatched".into(),
                        attempt: 1,
                        max_attempts: 3,
                        depends_on: vec![],
                        blocked_by: vec![],
                        unblocks: vec![],
                        summary: None,
                        feedback: None,
                        error: None,
                        worker_branch: Some("worker/build-ui".into()),
                        delegation_id: Some("req-ui".into()),
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: vec![],
                        next_action: "running".into(),
                    },
                    PlanSnapshotTask {
                        task_id: "api".into(),
                        task_name: "api".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-api".into()),
                        issue_title: None,
                        status: "failed".into(),
                        attempt: 2,
                        max_attempts: 3,
                        depends_on: vec![],
                        blocked_by: vec![],
                        unblocks: vec![],
                        summary: None,
                        feedback: Some("retry requested".into()),
                        error: Some("tests failed".into()),
                        worker_branch: None,
                        delegation_id: None,
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: vec![],
                        next_action: "retry".into(),
                    },
                    PlanSnapshotTask {
                        task_id: "release".into(),
                        task_name: "release".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-release".into()),
                        issue_title: None,
                        status: "pending".into(),
                        attempt: 0,
                        max_attempts: 3,
                        depends_on: vec!["lint".into()],
                        blocked_by: vec!["lint".into()],
                        unblocks: vec![],
                        summary: None,
                        feedback: None,
                        error: None,
                        worker_branch: None,
                        delegation_id: None,
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: vec![],
                        next_action: "wait".into(),
                    },
                ],
                owner_brain_session_id: None,
                owner_token: None,
                owner_acquired_at: None,
            }),
        },
    });
    store
}

fn lineage_with_long_running_ui() -> ExecutorLineage {
    let mut lineage = ExecutorLineage::new();
    let occurred_at = SystemTime::now()
        .checked_sub(Duration::from_secs(42 * 60))
        .unwrap();
    lineage.apply(&SpurEvent {
        occurred_at,
        seq: 0,
        body: SpurEventBody::ExecutorSpawned {
            id: "exec-ui".into(),
            parent_id: None,
            session_id: SessionId("worker-ui".into()),
            agent: "codex".into(),
            role: Role::Executor,
            task_spec: "build-ui".into(),
        },
    });
    lineage.apply(&SpurEvent {
        occurred_at,
        seq: 0,
        body: SpurEventBody::DelegationRequested {
            from: SessionId("brain-1".into()),
            to_agent: "codex".into(),
            task: "build-ui".into(),
            request_id: "req-ui".into(),
            delegation_plan: None,
            issue_id: Some("bd-build-ui".into()),
        },
    });
    lineage.apply(&SpurEvent {
        occurred_at,
        seq: 0,
        body: SpurEventBody::DelegationDispatched {
            from: SessionId("brain-1".into()),
            request_id: "req-ui".into(),
            executor_id: "exec-ui".into(),
        },
    });
    lineage.apply(&SpurEvent {
        occurred_at,
        seq: 0,
        body: SpurEventBody::ExecutorPhaseChanged {
            id: "exec-ui".into(),
            phase: LifecycleState::Running,
        },
    });
    lineage
}

fn visible_body_indices(buffer: &Buffer, prefix: &str) -> Vec<usize> {
    let mut indices = Vec::new();
    for y in 0..buffer.area.height {
        let row = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        if let Some(start) = row.find(prefix) {
            let number = row[start + prefix.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(index) = number.parse() {
                indices.push(index);
            }
        }
    }
    indices
}
