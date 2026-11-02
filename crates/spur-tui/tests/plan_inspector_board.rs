use ratatui::{backend::TestBackend, Terminal};
use spur_acp::{PlanSnapshotCounts, SessionId};
use spur_core::{ExecutorLineage, TrackedPlan, TrackedTask};
use spur_tui::components::plan_stage_board::{render_stacked_stage_groups, render_stage_board};

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
    render_mode(plan, width, height, selected, false)
}
fn render_mode(
    plan: &TrackedPlan,
    width: u16,
    height: u16,
    selected: usize,
    stacked: bool,
) -> String {
    let lineage = ExecutorLineage::new();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            if stacked {
                render_stacked_stage_groups(
                    frame,
                    frame.area(),
                    plan,
                    &format!("T{selected}"),
                    &lineage,
                    spur_tui::theme::fallback_theme(),
                );
            } else {
                render_stage_board(
                    frame,
                    frame.area(),
                    plan,
                    &format!("T{selected}"),
                    plan.tasks
                        .iter()
                        .find(|task| task.task_id == format!("T{selected}"))
                        .unwrap()
                        .stage_idx,
                    &lineage,
                    spur_tui::theme::fallback_theme(),
                )
            }
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
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

#[test]
fn crowded_lane_keeps_selected_title_and_metadata_visible_after_resize() {
    let tasks = (0..20)
        .map(|index| {
            let mut row = task(index);
            row.stage_idx = 0;
            row.attempt = 2;
            row
        })
        .collect();
    let plan = plan(tasks);
    for height in [8, 5] {
        for selected in [0, 10, 19] {
            let text = render(&plan, 90, height, selected);
            let title_y = text
                .lines()
                .position(|line| line.contains(&format!("▶ ○ label-{selected:04}")))
                .unwrap_or_else(|| panic!("selected row outside lane:\n{text}"));
            assert!(
                text.lines().nth(title_y + 1).unwrap().contains("retry 2/3"),
                "selected metadata clipped:\n{text}"
            );
            println!(
                "I2_GEOMETRY {}",
                serde_json::json!({"mode":"wide","height":height,"selected":selected,"y":title_y,"rows":2})
            );
        }
    }
}

#[test]
fn stacked_board_keeps_selected_stage_and_task_visible_after_resize() {
    let plan = plan((0..20).map(task).collect());
    for height in [8, 5] {
        for selected in [0, 10, 19] {
            let text = render_mode(&plan, 80, height, selected, true);
            let title_y = text
                .lines()
                .position(|line| line.contains(&format!("▶ ○ label-{selected:04}")))
                .unwrap_or_else(|| panic!("selected row outside stacked viewport:\n{text}"));
            assert!(
                text.contains(&format!("Stage {selected}")),
                "selected stage heading clipped:\n{text}"
            );
            println!(
                "I2_GEOMETRY {}",
                serde_json::json!({"mode":"stacked","height":height,"selected":selected,"y":title_y,"rows":1})
            );
        }
    }
}

#[test]
fn stacked_titles_keep_retry_metadata_visible() {
    for title in ["a long task title ".repeat(20), "界é👩‍💻 ".repeat(40)] {
        let mut row = task(0);
        row.task_name = title;
        row.attempt = 2;
        let plan = plan(vec![row]);
        for width in [40, 80] {
            let text = render_mode(&plan, width, 5, 0, true);
            let selected = text.lines().find(|line| line.contains('▶')).unwrap();
            assert!(
                selected.contains("retry 2/3") && selected.contains('…'),
                "long task title hid metadata or clipped silently at width {width}:\n{text}"
            );
            let cells = ratatui::text::Span::raw(selected).width();
            assert!(cells <= width as usize);
            println!(
                "U4_GEOMETRY {}",
                serde_json::json!({"width":width,"height":5,"cells":cells})
            );
        }
    }
}

#[test]
fn stacked_rows_prioritize_blocker_and_retry_chips() {
    let mut row = task(0);
    row.task_name = "a task with a long display name ".repeat(10);
    row.blocked_by = vec!["T1".into()];
    row.depends_on = vec!["T1".into()];
    row.attempt = 2;
    let plan = plan(vec![row]);
    let text = render_mode(&plan, 40, 5, 0, true);
    let selected = text.lines().find(|line| line.contains('▶')).unwrap();
    assert!(
        selected.contains("blocked:T1") && selected.contains("retry 2/3"),
        "priority chips lost to title or dependency history:\n{text}"
    );
    assert!(selected.contains('…'));
}
