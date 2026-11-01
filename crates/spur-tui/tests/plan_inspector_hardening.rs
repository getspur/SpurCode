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
