use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::time::SystemTime;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::{resolve_token, ColorDepth, Theme};
use spur_core::{ExecutorLineage, ExecutorNode, LifecycleState, TrackedPlan, TrackedTask};

const BLOCKED_ON_SETUP_CONFLICT_STATUS: &str = "blocked_on_setup_conflict";
// Ten readable title cells, four cursor/status cells, and two borders.
const MIN_STAGE_LANE_WIDTH: u16 = 10 + 4 + 2;

fn token(theme: &Theme, name: &str) -> ratatui::style::Color {
    resolve_token(theme, name, ColorDepth::Truecolor)
}

pub fn render_stage_board(
    frame: &mut Frame,
    area: Rect,
    plan: &TrackedPlan,
    selected_task_id: &str,
    selected_stage_idx: usize,
    lineage: &ExecutorLineage,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let stage_count = max_stage(plan) + 1;
    let visible_count = stage_count.min(usize::from(area.width / MIN_STAGE_LANE_WIDTH).max(1));
    let first_stage = selected_stage_idx
        .min(stage_count - 1)
        .saturating_sub(visible_count / 2)
        .min(stage_count - visible_count);
    let lane_area = if visible_count < stage_count {
        frame.render_widget(
            Paragraph::new(format!(
                "Stages {}-{}/{}{}{}",
                first_stage,
                first_stage + visible_count - 1,
                stage_count,
                if first_stage > 0 { " ←" } else { "" },
                if first_stage + visible_count < stage_count {
                    " →"
                } else {
                    ""
                },
            )),
            Rect::new(area.x, area.y, area.width, 1),
        );
        Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        )
    } else {
        area
    };
    let constraints = vec![Constraint::Ratio(1, visible_count as u32); visible_count];
    let areas = Layout::horizontal(constraints).split(lane_area);

    for (lane_idx, stage_area) in areas.iter().enumerate() {
        let stage_idx = first_stage + lane_idx;
        let tasks = tasks_in_stage(plan, stage_idx);
        let mut lines = Vec::new();
        let mut selected_rows = None;

        if tasks.is_empty() {
            lines.push(Line::from("(empty)"));
        } else {
            for task in tasks {
                let selected = task.task_id == selected_task_id;
                let start_row = lines.len();
                let title_line = vec![
                    Span::styled(
                        if selected { "▶ " } else { "  " },
                        if selected {
                            Style::default()
                                .fg(token(theme, "plan_inspector.board.selection.fg"))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(
                        status_glyph(&task.status),
                        status_style(theme, &task.status),
                    ),
                    Span::raw(" "),
                    Span::raw(fit_stage_label(
                        task.issue_title.as_ref().unwrap_or(&task.task_name),
                        stage_area.width.saturating_sub(6) as usize,
                    )),
                ];
                lines.push(Line::from(title_line));

                let meta = task_meta_chips(task, lineage, theme);
                if !meta.is_empty() {
                    let mut meta_line = vec![Span::raw("    ")];
                    for (i, span) in meta.into_iter().enumerate() {
                        if i > 0 {
                            meta_line.push(Span::raw("  "));
                        }
                        meta_line.push(span);
                    }
                    lines.push(Line::from(meta_line));
                }
                if selected {
                    selected_rows = Some((start_row, lines.len()));
                }
                lines.push(Line::raw(""));
            }
        }

        let is_active_stage = stage_idx == selected_stage_idx;
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Stage {}", stage_idx))
            .border_style(if is_active_stage {
                Style::default().fg(token(theme, "plan_inspector.board.stage.active.fg"))
            } else {
                Style::default().fg(token(theme, "plan_inspector.board.stage.inactive.fg"))
            })
            .title_style(if is_active_stage {
                Style::default()
                    .fg(token(theme, "plan_inspector.board.stage.active.fg"))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(token(theme, "plan_inspector.board.stage.inactive.fg"))
            });
        let offset = selection_offset(selected_rows, stage_area.height.saturating_sub(2) as usize);
        frame.render_widget(
            Paragraph::new(lines.into_iter().skip(offset).collect::<Vec<_>>()).block(block),
            *stage_area,
        );
    }
}

fn fit_stage_label(value: &str, width: usize) -> String {
    if Span::raw(value).width() <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let cells = Span::raw(grapheme).width();
        if used + cells > width - 1 {
            break;
        }
        result.push_str(grapheme);
        used += cells;
    }
    result.push('…');
    result
}

pub fn render_stacked_stage_groups(
    frame: &mut Frame,
    area: Rect,
    plan: &TrackedPlan,
    selected_task_id: &str,
    lineage: &ExecutorLineage,
    theme: &Theme,
) {
    let mut lines = Vec::new();
    let mut selected_rows = None;
    let active_stage_idx = plan
        .tasks
        .iter()
        .find(|task| task.task_id == selected_task_id)
        .map(|task| task.stage_idx);

    for stage_idx in 0..=max_stage(plan) {
        if stage_idx > 0 {
            lines.push(Line::from(Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(token(theme, "plan_inspector.board.stage.inactive.fg")),
            )));
        }
        let is_active_stage = active_stage_idx == Some(stage_idx);
        lines.push(Line::from(Span::styled(
            format!("Stage {}", stage_idx),
            if is_active_stage {
                Style::default()
                    .fg(token(theme, "plan_inspector.board.stage.active.fg"))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(token(theme, "plan_inspector.board.stage.inactive.fg"))
            },
        )));
        for task in tasks_in_stage(plan, stage_idx) {
            let selected = task.task_id == selected_task_id;
            let start_row = lines.len();
            let mut task_line = vec![
                Span::styled(
                    if selected { "▶ " } else { "  " },
                    if selected {
                        Style::default()
                            .fg(token(theme, "plan_inspector.board.selection.fg"))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    status_glyph(&task.status),
                    status_style(theme, &task.status),
                ),
                Span::raw(" "),
                Span::raw(task.task_name.clone()),
            ];
            let meta = task_meta_chips(task, lineage, theme);
            if !meta.is_empty() {
                task_line.push(Span::raw("  "));
                for (i, span) in meta.into_iter().enumerate() {
                    if i > 0 {
                        task_line.push(Span::raw("  "));
                    }
                    task_line.push(span);
                }
            }
            lines.push(Line::from(task_line));
            if selected {
                selected_rows = Some((start_row, lines.len()));
            }
        }
        lines.push(Line::raw(""));
    }

    let offset = selection_offset(selected_rows, area.height.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(offset).collect::<Vec<_>>()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Plan board")
                .border_style(
                    Style::default().fg(token(theme, "plan_inspector.board.stage.inactive.fg")),
                )
                .title_style(
                    Style::default().fg(token(theme, "plan_inspector.board.stage.active.fg")),
                ),
        ),
        area,
    );
}

fn selection_offset(selected_rows: Option<(usize, usize)>, visible_rows: usize) -> usize {
    selected_rows
        .map(|(start, end)| end.saturating_sub(visible_rows).min(start))
        .unwrap_or(0)
}

pub(crate) fn stage_grouped_tasks(plan: &TrackedPlan) -> Vec<&TrackedTask> {
    let mut ordered = Vec::new();
    for stage_idx in 0..=max_stage(plan) {
        ordered.extend(tasks_in_stage(plan, stage_idx));
    }
    ordered
}

pub fn preferred_live_node<'a>(
    lineage: &'a ExecutorLineage,
    issue_id: &str,
) -> Option<&'a ExecutorNode> {
    lineage
        .nodes_for_issue(issue_id)
        .into_iter()
        .max_by_key(|node| {
            (
                is_live_phase(node.phase),
                node.last_event_at
                    .or_else(|| node.current_attempt().map(|attempt| attempt.started_at))
                    .unwrap_or(SystemTime::UNIX_EPOCH),
                node.id.0.clone(),
            )
        })
}

fn max_stage(plan: &TrackedPlan) -> usize {
    plan.tasks
        .iter()
        .map(|task| task.stage_idx)
        .max()
        .unwrap_or(0)
}

fn tasks_in_stage(plan: &TrackedPlan, stage_idx: usize) -> Vec<&TrackedTask> {
    plan.tasks
        .iter()
        .filter(|task| task.stage_idx == stage_idx)
        .collect()
}

fn status_glyph(status: &str) -> &str {
    match status {
        "pending" => "○",
        "ready" => "◐",
        "dispatched" => "◉",
        "awaiting_review" => "◈",
        "approved" => "✓",
        "rejected" => "⊘",
        "failed" => "✕",
        "cancelled" => "⊝",
        "superseded" => "⇢",
        BLOCKED_ON_SETUP_CONFLICT_STATUS => "⦿",
        _ => "?",
    }
}

fn status_style(theme: &Theme, status: &str) -> Style {
    match status {
        "pending" => Style::default().fg(token(theme, "plan_inspector.status.pending.fg")),
        "ready" => Style::default().fg(token(theme, "plan_inspector.status.ready.fg")),
        "dispatched" => Style::default()
            .fg(token(theme, "plan_inspector.status.dispatched.fg"))
            .add_modifier(Modifier::BOLD),
        "awaiting_review" => Style::default()
            .fg(token(theme, "plan_inspector.status.awaiting_review.fg"))
            .add_modifier(Modifier::BOLD),
        "approved" => Style::default()
            .fg(token(theme, "plan_inspector.status.approved.fg"))
            .add_modifier(Modifier::BOLD),
        "rejected" => Style::default()
            .fg(token(theme, "plan_inspector.status.rejected.fg"))
            .add_modifier(Modifier::BOLD),
        "failed" => Style::default()
            .fg(token(theme, "plan_inspector.status.failed.fg"))
            .add_modifier(Modifier::BOLD),
        BLOCKED_ON_SETUP_CONFLICT_STATUS => Style::default()
            .fg(token(theme, "plan_inspector.status.blocked.fg"))
            .bg(token(theme, "plan_inspector.status.blocked.bg"))
            .add_modifier(Modifier::BOLD),
        "cancelled" => Style::default().fg(token(theme, "plan_inspector.status.cancelled.fg")),
        "superseded" => Style::default().fg(token(theme, "plan_inspector.status.unknown.fg")),
        _ => Style::default(),
    }
}

fn task_meta_chips<'a>(
    task: &TrackedTask,
    lineage: &ExecutorLineage,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut chips = Vec::new();

    // Live worker link (agent + phase)
    if let Some(issue_id) = task.issue_id.as_deref() {
        if let Some(node) = preferred_live_node(lineage, issue_id) {
            chips.push(Span::styled(
                format!("{}:{}", node.agent, phase_label(node.phase)),
                Style::default().fg(token(theme, "plan_inspector.chip.live.fg")),
            ));
        }
    }

    // Blocked indicator
    if !task.blocked_by.is_empty() {
        let label = if task.blocked_by.len() == 1 {
            format!("blocked:{}", task.blocked_by[0])
        } else {
            format!("blocked:{} deps", task.blocked_by.len())
        };
        chips.push(Span::styled(
            label,
            Style::default().fg(token(theme, "plan_inspector.chip.blocked.fg")),
        ));
    }

    if let Some(label) = setup_conflict_label(task) {
        chips.push(Span::styled(
            label,
            Style::default().fg(token(theme, "plan_inspector.chip.blocked.fg")),
        ));
    }

    // Dependency hint
    if !task.depends_on.is_empty() {
        let label = if task.depends_on.len() == 1 {
            format!("↑{}", task.depends_on[0])
        } else {
            format!("↑{} deps", task.depends_on.len())
        };
        chips.push(Span::styled(
            label,
            Style::default().fg(token(theme, "plan_inspector.chip.depends.fg")),
        ));
    }

    // Attempt / retry indicator
    if task.attempt > 1 {
        chips.push(Span::styled(
            format!("retry {}/{}", task.attempt, task.max_attempts),
            Style::default().fg(token(theme, "plan_inspector.chip.retry.fg")),
        ));
    }

    chips
}

fn setup_conflict_label(task: &TrackedTask) -> Option<String> {
    if task.status != BLOCKED_ON_SETUP_CONFLICT_STATUS {
        return None;
    }

    let dep_task_id = task
        .summary
        .as_deref()
        .and_then(setup_conflict_dep_task_id)
        .unwrap_or("dependency");
    let file_count = setup_conflict_file_count(task);
    let noun = if file_count == 1 { "file" } else { "files" };
    let verb = if file_count == 1 {
        "conflicts"
    } else {
        "conflict"
    };

    Some(format!("{file_count} {noun} {verb} with {dep_task_id}"))
}

fn setup_conflict_dep_task_id(summary: &str) -> Option<&str> {
    summary
        .strip_prefix("Setup overlay conflict applying ")
        .and_then(|rest| rest.split_once(':'))
        .map(|(dep_task_id, _)| dep_task_id.trim())
        .filter(|dep_task_id| !dep_task_id.is_empty())
}

fn setup_conflict_file_count(task: &TrackedTask) -> usize {
    task.error
        .as_deref()
        .map(|files| {
            files
                .split(',')
                .filter(|file| !file.trim().is_empty())
                .count()
        })
        .filter(|count| *count > 0)
        .or_else(|| {
            task.summary
                .as_deref()
                .and_then(setup_conflict_file_count_from_summary)
        })
        .unwrap_or(0)
}

fn setup_conflict_file_count_from_summary(summary: &str) -> Option<usize> {
    summary
        .rsplit_once(':')
        .and_then(|(_, count)| count.split_whitespace().next())
        .and_then(|count| count.parse().ok())
}

fn is_live_phase(phase: LifecycleState) -> bool {
    !matches!(
        phase,
        LifecycleState::Succeeded | LifecycleState::Failed | LifecycleState::Cancelled
    )
}

fn phase_label(phase: LifecycleState) -> &'static str {
    match phase {
        LifecycleState::Spawning => "spawn",
        LifecycleState::Running => "run",
        LifecycleState::AwaitingReview => "review",
        LifecycleState::Resuming => "resume",
        LifecycleState::Succeeded => "done",
        LifecycleState::Failed => "fail",
        LifecycleState::Cancelled => "stop",
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, style::Style, Terminal};
    use spur_acp::{PlanSnapshotCounts, SessionId};
    use spur_core::{ExecutorLineage, TrackedPlan, TrackedTask};

    use super::{render_stacked_stage_groups, render_stage_board};
    use crate::theme::load_built_in;

    fn task(task_id: &str, status: &str) -> TrackedTask {
        TrackedTask {
            task_id: task_id.into(),
            task_name: format!("{task_id} task"),
            agent: "codex".into(),
            issue_id: Some(format!("bd-1dwm.{task_id}")),
            issue_title: None,
            status: status.into(),
            attempt: 1,
            max_attempts: 3,
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            unblocks: Vec::new(),
            summary: None,
            feedback: None,
            error: None,
            worker_branch: None,
            delegation_id: None,
            diff_summary: None,
            mutation_id: None,
            superseded_by: Vec::new(),
            next_action: "wait".into(),
            stage_idx: 0,
        }
    }

    fn rendered_buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut rendered = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                rendered.push_str(buf[(x, y)].symbol());
            }
            rendered.push('\n');
        }
        rendered
    }

    fn style_for_cell_run(terminal: &Terminal<TestBackend>, needle: &str) -> Option<Style> {
        let buf = terminal.backend().buffer();
        for y in 0..buf.area.height {
            let mut row = String::new();
            let mut cell_x_by_byte = Vec::new();
            for x in 0..buf.area.width {
                cell_x_by_byte.push((row.len(), x));
                row.push_str(buf[(x, y)].symbol());
            }
            if let Some(start) = row.find(needle) {
                let x = cell_x_by_byte
                    .iter()
                    .find_map(|(byte_idx, x)| (*byte_idx == start).then_some(*x))?;
                return Some(buf[(x, y)].style());
            }
        }
        None
    }

    fn plan_with_tasks(tasks: Vec<TrackedTask>) -> TrackedPlan {
        TrackedPlan {
            session_id: SessionId("brain-1".into()),
            plan_id: "bd-1dwm".into(),
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

    #[test]
    fn plan_stage_board_renders_blocked_on_setup_conflict_distinct_from_failed() {
        let mut blocked = task("T2", "blocked_on_setup_conflict");
        blocked.summary = Some("Setup overlay conflict applying T1: 2 file(s)".into());
        blocked.error = Some("crates/spur-core/src/lib.rs, crates/spur-tui/src/lib.rs".into());

        let mut failed = task("T3", "failed");
        failed.error = Some("worker failed".into());

        let plan = plan_with_tasks(vec![blocked, failed]);
        let lineage = ExecutorLineage::new();
        let backend = TestBackend::new(140, 8);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let theme = load_built_in("dark").expect("built-in dark theme");

        terminal
            .draw(|frame| render_stage_board(frame, frame.area(), &plan, "T2", 0, &lineage, &theme))
            .expect("render stage board");

        let rendered = rendered_buffer_text(&terminal);
        assert!(rendered.contains("⦿"), "rendered: {rendered}");
        assert!(
            rendered.contains("2 files conflict with T1"),
            "rendered: {rendered}"
        );
        assert!(rendered.contains("✕"), "rendered: {rendered}");

        let blocked_style = style_for_cell_run(&terminal, "⦿").expect("blocked style");
        let failed_style = style_for_cell_run(&terminal, "✕").expect("failed style");
        assert_ne!(blocked_style, failed_style);
    }

    #[test]
    fn plan_stage_board_renders_every_status_glyph_without_panics() {
        let statuses_and_glyphs = [
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
            ("unknown_status", "?"),
        ];
        let tasks = statuses_and_glyphs
            .iter()
            .enumerate()
            .map(|(idx, (status, _))| {
                let mut task = task(&format!("T{idx}"), status);
                task.stage_idx = idx;
                task
            })
            .collect();

        let plan = plan_with_tasks(tasks);
        let lineage = ExecutorLineage::new();
        let backend = TestBackend::new(220, 10);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let theme = load_built_in("dark").expect("built-in dark theme");

        terminal
            .draw(|frame| render_stage_board(frame, frame.area(), &plan, "T0", 0, &lineage, &theme))
            .expect("render stage board");

        let rendered = rendered_buffer_text(&terminal);
        for (_, glyph) in statuses_and_glyphs {
            assert!(
                rendered.contains(glyph),
                "missing glyph {glyph}; rendered: {rendered}"
            );
        }
    }

    #[test]
    fn stacked_stage_groups_render_lightweight_stage_separators() {
        let mut first = task("T1", "pending");
        first.stage_idx = 0;
        let mut second = task("T2", "ready");
        second.stage_idx = 1;
        let mut third = task("T3", "approved");
        third.stage_idx = 2;

        let plan = plan_with_tasks(vec![first, second, third]);
        let lineage = ExecutorLineage::new();
        let backend = TestBackend::new(48, 12);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let theme = load_built_in("dark").expect("built-in dark theme");

        terminal
            .draw(|frame| {
                render_stacked_stage_groups(frame, frame.area(), &plan, "T2", &lineage, &theme)
            })
            .expect("render stacked stage groups");

        let rendered = rendered_buffer_text(&terminal);
        let separator_rows = rendered
            .lines()
            .filter(|line| {
                line.starts_with('│') && line.chars().filter(|ch| *ch == '─').count() >= 8
            })
            .count();
        assert_eq!(separator_rows, 2, "rendered: {rendered}");
        assert!(rendered.contains("Stage 0"), "rendered: {rendered}");
        assert!(rendered.contains("Stage 1"), "rendered: {rendered}");
        assert!(rendered.contains("Stage 2"), "rendered: {rendered}");
    }
}
