use ratatui::{backend::TestBackend, Terminal};
use spur_acp::{PlanSnapshotCounts, SessionId};
use spur_core::{ExecutorLineage, TrackedPlan, TrackedTask};
use spur_tui::components::plan_stage_board::render_stage_board;

fn task(index: usize) -> TrackedTask {
    TrackedTask {
        task_id: format!("T{index}"),
        task_name: format!("label-{index:04}"),
        agent: "codex".into(),
        issue_id: None,
        issue_title: None,
        status: "pending".into(),
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
        stage_idx: index,
    }
}
fn plan(tasks: Vec<TrackedTask>) -> TrackedPlan {
    TrackedPlan {
        session_id: SessionId("board-brain".into()),
        plan_id: "board-plan".into(),
        epic_id: None,
        status: "running".into(),
        progress: "0 reviewed".into(),
        next_action: "inspect".into(),
        ready_to_merge: false,
        owner_brain_session_id: None,
        counts: PlanSnapshotCounts::default(),
        tasks,
        updated_at: std::time::SystemTime::UNIX_EPOCH,
    }
}
fn render(plan: &TrackedPlan, width: u16, height: u16, selected: usize) -> String {
    let lineage = ExecutorLineage::new();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            render_stage_board(
                frame,
                frame.area(),
                plan,
                &format!("T{selected}"),
                selected,
                &lineage,
                spur_tui::theme::fallback_theme(),
            )
        })
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
fn stage_windows_keep_selected_title_and_heading_readable() {
    let plan = plan((0..18).map(task).collect());
    for width in [90, 120, 200] {
        for selected in [0, 9, 17] {
            let text = render(&plan, width, 20, selected);
            let header_y = text.lines().position(|line| line.starts_with('┌')).unwrap();
            let header = text.lines().nth(header_y).unwrap();
            let starts: Vec<_> = header
                .chars()
                .enumerate()
                .filter_map(|(x, ch)| (ch == '┌').then_some(x))
                .collect();
            let ends: Vec<_> = header
                .chars()
                .enumerate()
                .filter_map(|(x, ch)| (ch == '┐').then_some(x))
                .collect();
            let rectangles: Vec<_> = starts
                .iter()
                .zip(&ends)
                .map(|(&x, &end)| {
                    assert!(
                        end + 1 - x >= 16,
                        "lane loses the solved minimum title budget"
                    );
                    serde_json::json!({"x":x,"y":header_y,"width":end+1-x,"height":20-header_y})
                })
                .collect();
            if selected == 17 {
                println!(
                    "I1_GEOMETRY {}",
                    serde_json::json!({"width":width,"height":20,"rectangles":rectangles})
                );
            }
            assert!(
                text.contains(&format!("label-{selected:04}")),
                "selected title lost:\n{text}"
            );
            assert!(
                text.contains(&format!("Stage {selected}")),
                "stage heading lost:\n{text}"
            );
            assert!(
                text.contains("/18"),
                "stage window position missing:\n{text}"
            );
        }
    }
    for width in 0..16 {
        render(&plan, width, 3, 17);
    }
}
#[test]
fn long_stage_titles_show_visible_elision() {
    let mut row = task(0);
    row.task_name = "a task title longer than the entire lane".into();
    let plan = plan(vec![row]);
    assert!(
        render(&plan, 16, 6, 0).contains('…'),
        "clipping must show an ellipsis"
    );
}
