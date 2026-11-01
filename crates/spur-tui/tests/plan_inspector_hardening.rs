use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[test]
fn next_action_survives_terminal_retry_history_at_wide_and_narrow_widths() {
    for status in ["approved", "cancelled", "superseded"] {
        let mut row = task("historical", status);
        row.attempt = 2;
        let fixture = Fixture::new(snapshot(vec![row], PlanSnapshotCounts::default()));
        for width in [80, 200] {
            let text = fixture.render(&mut fixture.view(), width, 24);
            let header = text.lines().take(4).collect::<Vec<_>>().join("\n");
            assert!(
                header.contains("next: Review next task"),
                "next action replaced by history:\n{text}"
            );
            assert!(
                !header.contains("risk:") && !header.contains("retry 2/3"),
                "terminal retry is not a current risk:\n{text}"
            );
        }
    }
}
#[test]
fn shared_failed_blocker_is_reported_once_without_an_invented_age() {
    let failed = task("failed-lint", "failed");
    let children = ["child-a", "child-b"].map(|id| {
        let mut row = task(id, "pending");
        row.depends_on = vec!["failed-lint".into()];
        row.blocked_by = row.depends_on.clone();
        row
    });
    let fixture = Fixture::new(snapshot(
        vec![failed, children[0].clone(), children[1].clone()],
        PlanSnapshotCounts {
            failed: 1,
            pending: 2,
            ..Default::default()
        },
    ));
    for width in [80, 200] {
        let text = fixture.render(&mut fixture.view(), width, 24);
        let header = text.lines().take(4).collect::<Vec<_>>().join("\n");
        assert_eq!(
            header.matches("failed-lint error").count(),
            1,
            "duplicate blocker risk:\n{text}"
        );
        assert!(
            !header.contains("ago"),
            "snapshot refresh is not a failure timestamp:\n{text}"
        );
        assert!(
            header.contains("next: Review next task"),
            "risk must preserve next action:\n{text}"
        );
        assert!(header.contains("risk: failed-lint error"));
    }
}
fn add_worker(fixture: &mut Fixture, phase: spur_acp::LifecycleState) {
    use spur_acp::Role;
    for event in [
        SpurEventBody::ExecutorSpawned {
            id: "audit-exec".into(),
            parent_id: None,
            session_id: SessionId("audit-worker".into()),
            agent: "codex".into(),
            role: Role::Executor,
            task_spec: "task".into(),
        },
        SpurEventBody::DelegationRequested {
            from: SessionId("audit-brain".into()),
            to_agent: "codex".into(),
            task: "task".into(),
            request_id: "audit-request".into(),
            delegation_plan: None,
            issue_id: Some("bd-worker".into()),
        },
        SpurEventBody::DelegationDispatched {
            from: SessionId("audit-brain".into()),
            request_id: "audit-request".into(),
            executor_id: "audit-exec".into(),
        },
        SpurEventBody::ExecutorPhaseChanged {
            id: "audit-exec".into(),
            phase,
        },
    ] {
        fixture.lineage.apply(&SpurEvent::now(event));
    }
}
#[test]
fn terminal_worker_history_is_distinct_from_live_execution() {
    use spur_acp::LifecycleState::*;
    for (phase, live) in [
        (Spawning, true),
        (Running, true),
        (AwaitingReview, true),
        (Resuming, true),
        (Succeeded, false),
        (Failed, false),
        (Cancelled, false),
    ] {
        let mut row = task("task-a", if live { "dispatched" } else { "approved" });
        row.issue_id = Some("bd-worker".into());
        let mut fixture = Fixture::new(snapshot(vec![row], PlanSnapshotCounts::default()));
        add_worker(&mut fixture, phase);
        let text = fixture.render(&mut fixture.view(), 200, 24);
        let expected = format!(
            "{} codex {:?}",
            if live { "live" } else { "last worker" },
            phase
        )
        .to_lowercase();
        assert!(
            text.contains(&expected),
            "incorrect worker activity label, expected {expected}:\n{text}"
        );
        if !live {
            assert!(
                !text.contains("live codex"),
                "terminal history labeled live:\n{text}"
            );
        }
    }
}
#[test]
fn lifecycle_counts_are_disjoint_and_dependency_counts_are_labeled() {
    let mut pending = task("waiting", "pending");
    pending.blocked_by = vec!["external".into()];
    pending.depends_on = pending.blocked_by.clone();
    let fixture = Fixture::new(snapshot(
        vec![
            task("done", "approved"),
            pending,
            task("ready", "ready"),
            task("cancel", "cancelled"),
            task("old", "superseded"),
            task("conflict", "blocked_on_setup_conflict"),
        ],
        PlanSnapshotCounts {
            approved: 1,
            pending: 2,
            ready: 1,
            cancelled: 2,
            ..Default::default()
        },
    ));
    let text = fixture.render(&mut fixture.view(), 200, 24);
    let header = text.lines().take(4).collect::<Vec<_>>().join("\n");
    for expected in [
        "1 pending",
        "1 ready",
        "1 approved",
        "1 cancelled",
        "1 superseded",
        "1 setup conflict",
        "deps blocked: 1",
        "1 / 6 approved",
    ] {
        assert!(
            header.contains(expected),
            "missing disjoint count {expected}:\n{text}"
        );
    }
    assert!(!header.contains("2 cancelled") && !header.contains("2 pending"));
}
#[test]
fn setup_conflict_is_blocked_without_dependency_blockers() {
    let mut conflict = task("conflict", "blocked_on_setup_conflict");
    conflict.summary = Some("Setup overlay conflict applying parent: 1 file".into());
    conflict.error = Some("src/conflicted.rs".into());
    let fixture = Fixture::new(snapshot(
        vec![conflict],
        PlanSnapshotCounts {
            pending: 1,
            ..Default::default()
        },
    ));
    let text = fixture.render(&mut fixture.view(), 200, 24);
    let header = text.lines().take(4).collect::<Vec<_>>().join("\n");
    assert!(
        header.contains("1 setup conflict"),
        "setup conflict missing from attention counts:\n{text}"
    );
    assert!(
        text.contains("BLOCKED setup conflict"),
        "setup conflict lacks a reason banner:\n{text}"
    );
    assert!(
        text.contains("src/conflicted.rs"),
        "conflict file must be visible:\n{text}"
    );
}
#[test]
fn ordinary_pending_and_dependency_wait_have_distinct_blocking_feedback() {
    for (blocked, expected) in [(false, false), (true, true)] {
        let mut row = task("pending-task", "pending");
        if blocked {
            row.depends_on = vec!["external-parent".into()];
            row.blocked_by = row.depends_on.clone();
        }
        let fixture = Fixture::new(snapshot(
            vec![row],
            PlanSnapshotCounts {
                pending: 1,
                ..Default::default()
            },
        ));
        let text = fixture.render(&mut fixture.view(), 200, 24);
        assert_eq!(
            text.contains("BLOCKED"),
            expected,
            "pending classification wrong:\n{text}"
        );
        assert!(
            !text.contains("setup conflict"),
            "dependency wait is not a setup conflict:\n{text}"
        );
        if blocked {
            assert!(text.contains("external-parent"));
        }
    }
}
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

fn task(id: &str, status: &str) -> PlanSnapshotTask {
    PlanSnapshotTask {
        task_id: id.into(),
        task_name: id.into(),
        agent: "codex".into(),
        issue_id: None,
        issue_title: None,
        status: status.into(),
        attempt: 1,
        max_attempts: 3,
        depends_on: vec![],
        blocked_by: vec![],
        unblocks: vec![],
        summary: None,
        feedback: None,
        error: None,
        worker_branch: None,
        delegation_id: None,
        diff_summary: None,
        mutation_id: None,
        superseded_by: vec![],
        next_action: String::new(),
    }
}
fn snapshot(tasks: Vec<PlanSnapshotTask>, counts: PlanSnapshotCounts) -> PlanSnapshot {
    PlanSnapshot {
        plan_id: "plan-audit".into(),
        epic_id: None,
        status: "running".into(),
        progress: "legacy progress text".into(),
        next_action: "Review next task".into(),
        ready_to_merge: false,
        counts,
        tasks,
        owner_brain_session_id: None,
        owner_token: None,
        owner_acquired_at: None,
    }
}
struct Fixture {
    plans: PlanProjectionStore,
    lineage: ExecutorLineage,
    synopsis: SessionSynopsisProjection,
}
impl Fixture {
    fn new(snapshot: PlanSnapshot) -> Self {
        let mut plans = PlanProjectionStore::default();
        plans.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: SessionId("audit-brain".into()),
            snapshot: Box::new(snapshot),
        }));
        Self {
            plans,
            lineage: ExecutorLineage::new(),
            synopsis: SessionSynopsisProjection::new(),
        }
    }
    fn ctx(&self) -> ViewContext<'_> {
        ViewContext {
            lineage: &self.lineage,
            plan_projection: &self.plans,
            synopsis: &self.synopsis,
            brain_status: &BrainStatus::Idle,
            flag_summary: None,
            tombstone: None,
            transient_hint_override: None,
            notebook_ready: false,
            theme: spur_tui::theme::fallback_theme(),
        }
    }
    fn view(&self) -> PlanInspectorView {
        PlanInspectorView::new(SessionId("audit-brain".into()))
    }
    fn render(&self, view: &mut PlanInspectorView, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area(), &self.ctx()))
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
    fn key(&self, view: &mut PlanInspectorView, key: KeyCode) -> Option<Action> {
        view.handle_key(KeyEvent::new(key, KeyModifiers::NONE), &self.ctx())
    }
}

#[test]
fn escalation_counts_in_progress_and_attention() {
    let fixture = Fixture::new(snapshot(
        vec![
            task("done", "approved"),
            task("attention", "escalated_to_brain"),
        ],
        PlanSnapshotCounts {
            approved: 1,
            escalated: 1,
            ..Default::default()
        },
    ));
    let text = fixture.render(&mut fixture.view(), 200, 24);
    assert!(
        text.contains("1 / 2"),
        "escalation missing from total:\n{text}"
    );
    assert!(
        text.contains("1 escalated"),
        "escalation needs an explicit attention label:\n{text}"
    );
    let fixture = Fixture::new(snapshot(
        vec![task("attention", "escalated_to_brain")],
        PlanSnapshotCounts {
            escalated: 1,
            ..Default::default()
        },
    ));
    let text = fixture.render(&mut fixture.view(), 200, 24);
    assert!(
        text.contains("0 / 1"),
        "escalation-only plan must have a real denominator:\n{text}"
    );
}

#[test]
fn every_backend_task_status_has_an_explicit_board_glyph() {
    for (status, glyph) in [
        ("pending", "○"),
        ("ready", "◐"),
        ("dispatched", "◉"),
        ("awaiting_review", "◈"),
        ("approved", "✓"),
        ("rejected", "⊘"),
        ("failed", "✕"),
        ("cancelled", "⊝"),
        ("superseded", "⇢"),
        ("blocked_on_setup_conflict", "⦿"),
        ("escalated_to_brain", "!"),
    ] {
        let fixture = Fixture::new(snapshot(
            vec![task("task-a", status)],
            PlanSnapshotCounts::default(),
        ));
        let text = fixture.render(&mut fixture.view(), 200, 24);
        assert!(
            text.contains(&format!("▶ {glyph} task-a")),
            "missing glyph for {status}:\n{text}"
        );
    }
}
