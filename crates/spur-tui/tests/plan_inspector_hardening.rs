use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
#[test]
fn selected_detail_prioritizes_action_and_keeps_verbose_metadata_accessible() {
    let mut row = task("task-short", "failed");
    row.task_name = "concise".into();
    row.issue_title = Some("CONTEXT: approved design notebook ".repeat(30));
    row.error = Some("fix the overlapping file".into());
    row.next_action = "retry after resolving conflict".into();
    row.summary = Some(format!(
        "FIRST_OUTPUT_LINE\nSECOND_OUTPUT_LINE\n{}\nOUTPUT_TAIL",
        "wrapped output ".repeat(200)
    ));
    row.worker_branch = Some(format!(
        "spur/worker/{}BRANCH_TAIL",
        "metadata/".repeat(100)
    ));
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            failed: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    let text = fixture.render(&mut view, 100, 16);
    assert!(
        text.contains("concise · codex"),
        "concise identity hidden by context:\n{text}"
    );
    for expected in [
        "error: fix the overlapping file",
        "next: retry after resolving conflict",
    ] {
        assert!(
            text.contains(expected),
            "actionable detail hidden: {expected}\n{text}"
        );
    }
    for needle in [
        "error: fix the overlapping file",
        "next: retry after resolving conflict",
    ] {
        let y = text.lines().position(|line| line.contains(needle)).unwrap();
        println!(
            "I9_GEOMETRY {}",
            serde_json::json!({"needle":needle,"x":53,"y":y,"width":needle.len(),"height":1})
        );
    }
    fixture.key(&mut view, KeyCode::Tab);
    fixture.key(&mut view, KeyCode::Char('G'));
    assert!(fixture.render(&mut view, 100, 16).contains("OUTPUT_TAIL"));
    fixture.key(&mut view, KeyCode::Char('e'));
    fixture.render(&mut view, 100, 16);
    fixture.key(&mut view, KeyCode::Char('G'));
    let expanded = fixture.render(&mut view, 100, 16);
    let pane = expanded
        .lines()
        .map(|line| {
            line.chars()
                .skip(53)
                .take(46)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        pane.contains("BRANCH_TAIL"),
        "expanded metadata remains reachable across wrapped rows:\n{expanded}"
    );
}
#[test]
fn multiline_task_output_preserves_source_line_boundaries() {
    let mut row = task("multiline", "approved");
    row.summary = Some("FIRST_OUTPUT_LINE\nSECOND_OUTPUT_LINE\nOUTPUT_TAIL".into());
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            approved: 1,
            ..Default::default()
        },
    ));
    let text = fixture.render(&mut fixture.view(), 200, 24);
    let first = text
        .lines()
        .position(|line| line.contains("FIRST_OUTPUT_LINE"))
        .unwrap();
    let second = text
        .lines()
        .position(|line| line.contains("SECOND_OUTPUT_LINE"))
        .unwrap();
    assert!(
        second > first,
        "multiline output flattened into one rendered line:\n{text}"
    );
}
#[test]
fn full_dependency_ids_expand_and_parent_child_navigation_cycles_from_origin() {
    let parents: Vec<_> = (0..3)
        .map(|i| format!("parent-{i:02}-very-long-identifier"))
        .collect();
    let children: Vec<_> = (0..3)
        .map(|i| format!("child-{i:02}-very-long-identifier"))
        .collect();
    let mut hub = task("hub", "pending");
    hub.depends_on = parents
        .iter()
        .cloned()
        .chain(["external-parent".into()])
        .collect();
    hub.blocked_by = parents[..2].to_vec();
    hub.unblocks = children.clone();
    let mut tasks: Vec<_> = parents.iter().map(|id| task(id, "approved")).collect();
    tasks.push(hub);
    tasks.extend(children.iter().map(|id| {
        let mut row = task(id, "pending");
        row.depends_on = vec!["hub".into()];
        row
    }));
    let fixture = Fixture::new(snapshot(
        tasks,
        PlanSnapshotCounts {
            approved: 3,
            pending: 4,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    fixture.render(&mut view, 100, 24);
    fixture.key(&mut view, KeyCode::Char('l'));
    fixture.key(&mut view, KeyCode::Char('e'));
    let mut seen = String::new();
    for _ in 0..50 {
        seen.push_str(&fixture.render(&mut view, 100, 24));
        fixture.key(&mut view, KeyCode::Down);
    }
    for id in parents.iter().chain(&children) {
        assert!(seen.contains(id), "expanded relationship ID missing: {id}");
    }
    fixture.key(&mut view, KeyCode::Char('p'));
    assert!(
        fixture
            .render(&mut view, 100, 24)
            .contains(&format!("task: {}", parents[0])),
        "p must select first parent"
    );
    fixture.key(&mut view, KeyCode::Char('p'));
    assert!(
        fixture
            .render(&mut view, 100, 24)
            .contains(&format!("task: {}", parents[1])),
        "p must cycle the original parent list"
    );
    fixture.key(&mut view, KeyCode::Char('p'));
    let action = fixture.key(&mut view, KeyCode::Char('p'));
    assert!(
        matches!(action,Some(Action::FlashHint { message }) if message.contains("external-parent"))
    );
    fixture.key(&mut view, KeyCode::Char('l'));
    fixture.key(&mut view, KeyCode::Char('n'));
    assert!(
        fixture
            .render(&mut view, 100, 24)
            .contains(&format!("task: {}", children[0])),
        "n must select first child"
    );
    fixture.key(&mut view, KeyCode::Char('n'));
    let text = fixture.render(&mut view, 100, 24);
    assert!(
        text.contains(&format!("task: {}", children[1])),
        "n must cycle original child list"
    );
    fixture.key(&mut view, KeyCode::Char('h'));
    fixture.key(&mut view, KeyCode::Char('b'));
    let text = fixture.render(&mut view, 100, 24);
    assert!(
        text.contains(&format!("task: {}", parents[0])),
        "b blocker navigation must remain available"
    );
    let empty = Fixture::new(snapshot(
        vec![task("alone", "pending")],
        PlanSnapshotCounts::default(),
    ));
    let mut view = empty.view();
    empty.render(&mut view, 100, 24);
    assert!(
        matches!(empty.key(&mut view,KeyCode::Char('p')),Some(Action::FlashHint { message }) if message.contains("no parents"))
    );
    assert!(
        matches!(empty.key(&mut view,KeyCode::Char('n')),Some(Action::FlashHint { message }) if message.contains("no children"))
    );
}
#[test]
fn compact_header_protects_status_progress_and_selection_at_each_geometry() {
    let tasks = (0..20)
        .map(|i| task(&format!("selected-{i:02}"), "pending"))
        .collect();
    let mut state = snapshot(
        tasks,
        PlanSnapshotCounts {
            pending: 20,
            ..Default::default()
        },
    );
    state.plan_id = "213e60f5-6427-4e53-bd64-7ad4ca6fe4c0".into();
    let fixture = Fixture::new(state);
    let mut view = fixture.view();
    for (width, height) in [(80, 24), (89, 24), (90, 24), (120, 32), (200, 50), (90, 8)] {
        fixture.render(&mut view, width, height);
        fixture.key(&mut view, KeyCode::Char('G'));
        let text = fixture.render(&mut view, width, height);
        let header = text.lines().take(3).collect::<Vec<_>>().join("\n");
        assert!(
            header.contains("RUNNING"),
            "status lost at {width}x{height}:\n{text}"
        );
        assert!(header.contains("0 / 20 approved"), "progress lost:\n{text}");
        assert!(
            header.contains("next: Review next task"),
            "next action lost:\n{text}"
        );
        assert!(
            !header.contains("213e60f5-6427-4e53-bd64-7ad4ca6fe4c0"),
            "full UUID crowds primary header"
        );
        assert!(
            text.contains("▶ ○ selected-19"),
            "selected task lost during resize:\n{text}"
        );
        let lines: Vec<_> = text.lines().collect();
        let mut rectangles = vec![];
        for (y, line) in lines.iter().enumerate() {
            let starts: Vec<_> = line
                .chars()
                .enumerate()
                .filter_map(|(x, c)| (c == '┌').then_some(x))
                .collect();
            let ends: Vec<_> = line
                .chars()
                .enumerate()
                .filter_map(|(x, c)| (c == '┐').then_some(x))
                .collect();
            for (&x, &end) in starts.iter().zip(&ends) {
                let bottom = (y + 1..lines.len())
                    .find(|&row| lines[row].chars().nth(x) == Some('└'))
                    .unwrap();
                rectangles
                    .push(serde_json::json!({"x":x,"y":y,"width":end+1-x,"height":bottom+1-y}));
            }
        }
        println!(
            "I8_GEOMETRY {}",
            serde_json::json!({"width":width,"height":height,"rectangles":rectangles})
        );
    }
    fixture.key(&mut view, KeyCode::Char('?'));
    let help = fixture.render(&mut view, 90, 24);
    assert!(
        help.contains("213e60f5-6427-4e53-bd64-7ad4ca6fe4c0"),
        "full plan identity must remain inspectable:\n{help}"
    );
    assert!(
        fixture.key(&mut view, KeyCode::Esc).is_none(),
        "Esc should close help before leaving inspector"
    );
}
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
        buffer_text(buffer, width, height)
    }
    fn render_streams(&self, view: &mut PlanInspectorView, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut streams = spur_tui::worker_streams::WorkerStreams::new();
        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut streams, &self.ctx())
            })
            .unwrap();
        buffer_text(terminal.backend().buffer(), width, height)
    }
    fn stream_key(&self, view: &mut PlanInspectorView, code: KeyCode) -> Option<Action> {
        let mut streams = spur_tui::worker_streams::WorkerStreams::new();
        view.handle_key_with_worker_streams(
            KeyEvent::new(code, KeyModifiers::NONE),
            &mut streams,
            &self.ctx(),
        )
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

#[test]
fn short_mixed_attention_counts_stay_visible_and_full_counts_are_inspectable() {
    let statuses = [
        "pending",
        "ready",
        "dispatched",
        "awaiting_review",
        "approved",
        "rejected",
        "failed",
        "cancelled",
        "superseded",
        "blocked_on_setup_conflict",
        "escalated_to_brain",
    ];
    let fixture = Fixture::new(snapshot(
        statuses
            .iter()
            .enumerate()
            .map(|(i, s)| task(&format!("T{i}"), s))
            .collect(),
        PlanSnapshotCounts {
            pending: 2,
            ready: 1,
            dispatched: 1,
            awaiting_review: 1,
            approved: 1,
            rejected: 1,
            failed: 1,
            cancelled: 2,
            escalated: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    let text = fixture.render(&mut view, 90, 8);
    let header = text.lines().take(3).collect::<Vec<_>>().join("\n");
    for label in ["1 escalated", "1 setup conflict", "1 failed"] {
        assert!(
            header.contains(label),
            "priority attention count hidden: {label}\n{text}"
        );
    }
    fixture.key(&mut view, KeyCode::Char('?'));
    let help = fixture
        .render(&mut view, 90, 24)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for label in [
        "1 pending",
        "1 ready",
        "1 dispatched",
        "1 awaiting_review",
        "1 approved",
        "1 rejected",
        "1 failed",
        "1 cancelled",
        "1 superseded",
        "1 setup conflict",
        "1 escalated",
    ] {
        assert!(
            help.contains(label),
            "complete count missing from help: {label}\n{help}"
        );
    }
}

fn buffer_text(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
    (0..height)
        .map(|y| {
            let mut row = String::new();
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += ratatui::text::Span::raw(symbol).width().max(1) as u16;
            }
            row
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn footer_matches_board_detail_issue_and_confirmation_ownership() {
    let mut row = task("task-a", "pending");
    row.issue_id = Some("bd-worker".into());
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            pending: 1,
            ..Default::default()
        },
    ));
    for width in [40, 80, 160, 240] {
        let mut view = fixture.view();
        let text = fixture.render(&mut view, width, 24);
        let footer = text.lines().last().unwrap();
        assert!(
            footer.contains("? help") && footer.contains("Esc"),
            "essential hints missing:\n{text}"
        );
        assert!(
            footer.contains("j/k: task"),
            "board footer must describe task navigation:\n{text}"
        );
        fixture.key(&mut view, KeyCode::Tab);
        let text = fixture.render(&mut view, width, 24);
        let footer = text.lines().last().unwrap();
        assert!(
            footer.contains("j/k: scroll") && !footer.contains("j/k: task"),
            "detail footer contradicts ownership:\n{text}"
        );
        assert!(text.contains("[focus]"), "focused detail must be visible");
        fixture.key(&mut view, KeyCode::Enter);
        let text = fixture.render(&mut view, width, 24);
        let footer = text.lines().last().unwrap();
        assert!(footer.contains("j/k: scroll") && !footer.contains("j/k: task"));
        fixture.key(&mut view, KeyCode::Esc);
        fixture.key(&mut view, KeyCode::Char('r'));
        let text = fixture.render(&mut view, width, 24);
        let footer = text.lines().last().unwrap();
        assert!(
            footer.contains("Enter") && footer.contains("Esc") && !footer.contains("j/k"),
            "confirmation footer leaks browse commands:\n{text}"
        );
        for code in ['e', 'p', 'n', '?'] {
            assert!(fixture.key(&mut view, KeyCode::Char(code)).is_none());
        }
        fixture.key(&mut view, KeyCode::Esc);
        assert!(fixture.render(&mut view, width, 24).contains("[focus]"));
    }
}
#[test]
fn help_scrolls_the_complete_status_legend_and_restores_detail_focus() {
    let fixture = Fixture::new(snapshot(
        vec![task("task-a", "pending")],
        PlanSnapshotCounts {
            pending: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    fixture.render(&mut view, 80, 16);
    fixture.key(&mut view, KeyCode::Tab);
    fixture.key(&mut view, KeyCode::Char('?'));
    let mut seen = String::new();
    for _ in 0..80 {
        seen.push_str(&fixture.render(&mut view, 80, 16));
        fixture.key(&mut view, KeyCode::Down);
    }
    for label in [
        "○ pending",
        "◐ ready",
        "◉ dispatched",
        "◈ awaiting_review",
        "✓ approved",
        "⊘ rejected",
        "✕ failed",
        "⊝ cancelled",
        "⇢ superseded",
        "⦿ setup conflict",
        "! escalated",
    ] {
        assert!(
            seen.contains(label),
            "status missing from scrollable legend: {label}"
        );
    }
    for hint in ["Tab", "p/n", "e:", "stage", "Plan:"] {
        assert!(seen.contains(hint), "help missing {hint}");
    }
    fixture.key(&mut view, KeyCode::Char('G'));
    let bottom = fixture.render(&mut view, 80, 16);
    fixture.key(&mut view, KeyCode::Up);
    assert_ne!(bottom, fixture.render(&mut view, 80, 16));
    assert!(fixture.key(&mut view, KeyCode::Esc).is_none());
    assert!(fixture.render(&mut view, 80, 16).contains("[focus]"));
    assert!(matches!(
        fixture.key(&mut view, KeyCode::Down),
        Some(Action::ScrollDown)
    ));
}
#[test]
fn stream_help_and_modal_dependency_keys_do_not_leak_into_browsing() {
    let mut row = task("task-a", "dispatched");
    row.issue_id = Some("bd-worker".into());
    row.delegation_id = Some("audit-request".into());
    row.unblocks = vec!["child".into()];
    let mut child = task("child", "pending");
    child.depends_on = vec!["task-a".into()];
    let mut fixture = Fixture::new(snapshot(
        vec![row, child],
        PlanSnapshotCounts {
            dispatched: 1,
            pending: 1,
            ..Default::default()
        },
    ));
    add_worker(&mut fixture, spur_acp::LifecycleState::Running);
    let mut view = fixture.view();
    fixture.render(&mut view, 120, 24);
    view.enter_stream_peek("audit-exec".into(), "task-a".into());
    let text = fixture.render_streams(&mut view, 120, 24);
    let footer = text.lines().last().unwrap();
    let narrow = fixture.render_streams(&mut view, 40, 24);
    assert!(
        narrow.contains("? help"),
        "narrow peek must retain help access:\n{narrow}"
    );
    assert!(
        footer.contains("j/k: scroll") && !footer.contains("j/k: task"),
        "stream footer leaks browsing:\n{text}"
    );
    for code in ['e', 'p', 'n'] {
        assert!(fixture.stream_key(&mut view, KeyCode::Char(code)).is_none());
    }
    fixture.stream_key(&mut view, KeyCode::Char('?'));
    assert!(fixture
        .render_streams(&mut view, 120, 24)
        .contains("Plan inspector help"));
    fixture.stream_key(&mut view, KeyCode::Esc);
    assert!(matches!(
        view.mode(),
        spur_tui::views::plan_inspector::PlanInspectorMode::StreamPeek { .. }
    ));
    fixture.stream_key(&mut view, KeyCode::Esc);
    let text = fixture.render(&mut view, 120, 24);
    assert!(!text.contains("Relationships (p parents"));
    assert!(text.lines().last().unwrap().contains("j/k: task"));
    fixture.key(&mut view, KeyCode::Enter);
    for code in ['e', 'p', 'n'] {
        assert!(fixture.key(&mut view, KeyCode::Char(code)).is_none());
    }
    fixture.key(&mut view, KeyCode::Esc);
    assert!(!fixture
        .render(&mut view, 120, 24)
        .contains("Relationships (p parents"));
    fixture.key(&mut view, KeyCode::Char('e'));
    assert!(fixture
        .render(&mut view, 120, 24)
        .contains("Relationships (p parents"));
    fixture.key(&mut view, KeyCode::Char('e'));
    assert!(!fixture
        .render(&mut view, 120, 24)
        .contains("Relationships (p parents"));
    fixture.key(&mut view, KeyCode::Char('e'));
    fixture.key(&mut view, KeyCode::Char('n'));
    let text = fixture.render(&mut view, 120, 24);
    assert!(text.contains("task: child") && !text.contains("Relationships (p parents"));
}
#[test]
fn long_next_action_shares_header_space_with_actionable_risk() {
    let mut child = task("child", "pending");
    child.depends_on = vec!["lint".into()];
    child.blocked_by = child.depends_on.clone();
    let mut state = snapshot(
        vec![task("lint", "failed"), child],
        PlanSnapshotCounts {
            failed: 1,
            pending: 1,
            ..Default::default()
        },
    );
    state.next_action = "Review the evidence and resolve the pending decision. ".repeat(8);
    let fixture = Fixture::new(state);
    let text = fixture.render(&mut fixture.view(), 80, 24);
    let header = text.lines().nth(1).unwrap();
    assert!(
        header.contains("next: Review") && header.contains("risk: lint error"),
        "long next action hides risk:\n{text}"
    );
}
#[test]
fn unicode_status_footer_preserves_essential_hints_within_cell_budget() {
    let mut row = task("任务-é-👩‍💻", "pending");
    row.depends_on = vec!["父任务-👩‍💻".into()];
    row.blocked_by = row.depends_on.clone();
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            pending: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::PlanCommandError {
            operation: "RetryPlanTask".into(),
            plan_id: Some("plan-audit".into()),
            error: "漢字é👩‍💻".repeat(30),
        }),
        &fixture.ctx(),
    );
    for width in [20, 40, 80, 160] {
        let text = fixture.render(&mut view, width, 24);
        let footer = text.lines().last().unwrap();
        assert!(
            footer.contains("Esc") && footer.contains("? help"),
            "Unicode status displaced essential controls:\n{text}"
        );
        assert!(ratatui::text::Span::raw(footer).width() <= width as usize);
        for line in text.lines() {
            assert!(ratatui::text::Span::raw(line).width() <= width as usize);
        }
        println!(
            "I10_FOOTER {}",
            serde_json::json!({"mode":"board","width":width,"cells":ratatui::text::Span::raw(footer).width()})
        );
    }
}

#[test]
fn cell_arabic_footer_keeps_controls_visible_under_contextual_width() {
    let fixture = Fixture::new(snapshot(
        vec![task("task-a", "pending")],
        PlanSnapshotCounts {
            pending: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::PlanCommandError {
            operation: "RetryPlanTask".into(),
            plan_id: Some("plan-audit".into()),
            error: "لا".repeat(80),
        }),
        &fixture.ctx(),
    );
    let text = fixture.render(&mut view, 160, 24);
    let footer = text.lines().last().unwrap();
    assert!(
        footer.contains("Esc close") && footer.contains("? help"),
        "contextual string width displaced essential controls:\n{text}"
    );
    let cells = footer
        .graphemes(true)
        .map(|g| ratatui::text::Span::raw(g).width())
        .sum::<usize>();
    assert!(cells <= 160);
    println!(
        "I10_FOOTER {}",
        serde_json::json!({"mode":"board-arabic","width":160,"cells":cells})
    );
}

#[test]
fn command_errors_remain_visible_at_compact_sizes() {
    let mut row = task("failed-task", "failed");
    row.issue_id = Some("bd-failed".into());
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            failed: 1,
            ..Default::default()
        },
    ));
    let mut view = fixture.view();
    let error = format!(
        "retry_task: {}ERROR_TAIL",
        "resolve the conflicting file first; ".repeat(12)
    );
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::PlanCommandError {
            operation: "RetryPlanTask".into(),
            plan_id: Some("plan-audit".into()),
            error,
        }),
        &fixture.ctx(),
    );
    for (width, height) in [(40, 8), (80, 8), (80, 24)] {
        let text = fixture.render(&mut view, width, height);
        let header = text.lines().nth(1).unwrap();
        assert!(
            header.contains("Error:") && header.contains("RetryPlanTask"),
            "command failure is invisible at {width}x{height}:\n{text}"
        );
        assert!(
            text.contains("▶ ✕ failed-task"),
            "error displaced the selected task:\n{text}"
        );
        println!(
            "U1_GEOMETRY {}",
            serde_json::json!({"width":width,"height":height,"error":{"x":0,"y":1,"width":ratatui::text::Span::raw(header.trim_end()).width(),"height":1}})
        );
    }
    view.handle_spur_event(
        &SpurEvent::now(SpurEventBody::LoopPaused {
            loop_id: "unrelated-loop".into(),
            by: "operator".into(),
        }),
        &fixture.ctx(),
    );
    assert!(
        fixture
            .render(&mut view, 40, 8)
            .lines()
            .nth(1)
            .unwrap()
            .contains("Error:"),
        "loop event erased the command failure"
    );
    fixture.key(&mut view, KeyCode::Char('?'));
    let mut help = String::new();
    for _ in 0..100 {
        help.push_str(&fixture.render(&mut view, 40, 8));
        fixture.key(&mut view, KeyCode::Down);
    }
    assert!(
        help.contains("ERROR_TAIL"),
        "full command error is not reachable in help"
    );
    fixture.key(&mut view, KeyCode::Esc);
    fixture.key(&mut view, KeyCode::Char('r'));
    assert!(matches!(
        fixture.key(&mut view, KeyCode::Enter),
        Some(Action::ResumePlan { .. })
    ));
    assert!(
        fixture
            .render(&mut view, 40, 8)
            .lines()
            .nth(1)
            .unwrap()
            .contains("next:"),
        "confirmed command retained a stale failure"
    );
}

#[test]
fn compact_footer_keeps_pane_switch_and_return_controls_visible() {
    let mut row = task("pane-task", "failed");
    row.issue_id = Some("bd-pane".into());
    let fixture = Fixture::new(snapshot(
        vec![row],
        PlanSnapshotCounts {
            failed: 1,
            ..Default::default()
        },
    ));
    for width in [40, 80] {
        for height in [8, 24] {
            let mut view = fixture.view();
            view.handle_spur_event(
                &SpurEvent::now(SpurEventBody::LoopPaused {
                    loop_id: "long-loop-name".into(),
                    by: "operator".into(),
                }),
                &fixture.ctx(),
            );
            let board = fixture.render(&mut view, width, height);
            let footer = board.lines().last().unwrap();
            assert!(
                footer.contains("Tab") && footer.contains("detail"),
                "board hid its pane-switch control at {width}x{height}:\n{board}"
            );
            assert!(
                footer.contains("Esc") && footer.contains("? help") && footer.contains("j/k: task")
            );
            println!(
                "U3_GEOMETRY {}",
                serde_json::json!({"mode":"board","width":width,"height":height,"cells":ratatui::text::Span::raw(footer.trim_end()).width()})
            );
            fixture.key(&mut view, KeyCode::Tab);
            let detail = fixture.render(&mut view, width, height);
            let footer = detail.lines().last().unwrap();
            assert!(
                footer.contains("Tab")
                    && footer.contains("board")
                    && footer.contains("j/k: scroll"),
                "detail hid its return control:\n{detail}"
            );
            println!(
                "U3_GEOMETRY {}",
                serde_json::json!({"mode":"detail","width":width,"height":height,"cells":ratatui::text::Span::raw(footer.trim_end()).width()})
            );
            fixture.key(&mut view, KeyCode::Enter);
            let issue = fixture.render(&mut view, width, height);
            let footer = issue.lines().last().unwrap();
            assert!(
                footer.contains("Enter")
                    && footer.contains("Esc")
                    && footer.contains("? help")
                    && footer.contains("j/k: scroll"),
                "issue hid its return control:\n{issue}"
            );
            println!(
                "U3_GEOMETRY {}",
                serde_json::json!({"mode":"issue","width":width,"height":height,"cells":ratatui::text::Span::raw(footer.trim_end()).width()})
            );
        }
    }
}
