use chrono::{DateTime, Utc};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use spur_core::{ExecutorNode, TrackedPlan, TrackedTask};
use spur_pm::Issue;

use crate::theme::{resolve_token, ColorDepth, Theme};

pub struct TaskDetailScroll {
    pub offset: usize,
    pub max_offset: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn render_task_detail(
    frame: &mut Frame,
    area: Rect,
    plan: &TrackedPlan,
    task: &TrackedTask,
    live_node: Option<&ExecutorNode>,
    issue_detail: Option<&Issue>,
    issue_detail_status: Option<&str>,
    scroll_offset: usize,
    theme: &Theme,
) -> TaskDetailScroll {
    let content_width = area.width.saturating_sub(2) as usize;
    let now = Utc::now();
    let mut lines = vec![hero_line(task, live_node, theme)];
    if task_is_blocked(task) {
        lines.push(blocked_banner(task, plan, content_width, theme));
    }
    lines.extend(dependency_strip(task, plan, content_width, theme));
    lines.push(execution_strip(task, theme));
    lines.push(kv(theme, "task", &task.task_id));
    if let Some(issue_id) = task.issue_id.as_deref() {
        lines.push(kv(theme, "issue", issue_id));
    }

    if task.summary.is_some()
        || task.feedback.is_some()
        || task.error.is_some()
        || task.diff_summary.is_some()
        || task.mutation_id.is_some()
        || !task.superseded_by.is_empty()
        || !task.next_action.is_empty()
    {
        lines.extend(section_header("Output", theme));
        if let Some(summary) = task.summary.as_deref() {
            lines.push(kv(theme, "summary", summary));
        }
        if let Some(feedback) = task.feedback.as_deref() {
            lines.push(kv(theme, "feedback", feedback));
        }
        if let Some(error) = task.error.as_deref() {
            lines.push(kv(theme, "error", error));
        }
        if let Some(diff_summary) = task.diff_summary.as_ref() {
            lines.push(kv(
                theme,
                "diff",
                &format!(
                    "{} files +{}/-{}",
                    diff_summary.files_changed, diff_summary.insertions, diff_summary.deletions
                ),
            ));
        }
        if let Some(mutation_id) = task.mutation_id.as_deref() {
            lines.push(kv(theme, "mutation", mutation_id));
        }
        if !task.superseded_by.is_empty() {
            lines.push(kv(theme, "superseded_by", &task.superseded_by.join(", ")));
        }
        if !task.next_action.is_empty() {
            lines.push(kv(theme, "next", &task.next_action));
        }
    }

    if let Some(status) = issue_detail_status {
        lines.extend(section_header("Issue", theme));
        lines.push(kv(theme, "status", status));
    }
    if let Some(issue) = issue_detail {
        // Essentials + Description first so the body is visible at scroll 0
        // in the narrow side-by-side detail pane (metadata alone can exceed
        // the viewport and push the description off-screen).
        lines.extend(section_header("Issue", theme));
        lines.push(kv(theme, "id", &issue.id));
        lines.push(kv(theme, "title", &issue.title));
        lines.push(kv(theme, "status", &issue.status));

        lines.extend(section_header("Description", theme));
        if issue.body.is_empty() {
            lines.push(kv(theme, "body", "(empty)"));
        } else {
            for line in issue.body.lines() {
                lines.push(Line::from(vec![Span::raw(line.to_string())]));
            }
        }

        lines.extend(section_header("Metadata", theme));
        lines.push(kv(
            theme,
            "source",
            match issue.source {
                spur_pm::PmSource::GitHub => "github",
                spur_pm::PmSource::Linear => "linear",
                spur_pm::PmSource::Plane => "plane",
                _ => "beads",
            },
        ));
        if let Some(priority) = issue.priority {
            lines.push(kv(theme, "priority", &format!("P{priority}")));
        }
        if let Some(issue_type) = issue.issue_type.as_deref() {
            lines.push(kv(theme, "type", issue_type));
        }
        lines.push(kv(
            theme,
            "assignee",
            issue.assignee.as_deref().unwrap_or("unassigned"),
        ));
        if let Some(due_at) = issue.due_at {
            lines.push(kv(theme, "due", &format_date_with_age(due_at, now)));
        }
        lines.push(kv(
            theme,
            "created",
            &format_date_with_age(issue.created_at, now),
        ));
        lines.push(kv(
            theme,
            "updated",
            &format_date_with_age(issue.updated_at, now),
        ));
        lines.extend(labels_block(&issue.labels, theme));
        if !issue.blocked_by.is_empty() {
            lines.push(kv(theme, "blocked_by", &issue.blocked_by.join(", ")));
        }
        if !issue.url.is_empty() {
            lines.push(kv(theme, "url", &issue.url));
        }
    }

    let visible_lines = area.height.saturating_sub(2) as usize;
    // Virtual rows use usize; only the visible slice reaches the u16 widget.
    let source: Vec<String> = lines.iter().map(ToString::to_string).collect();
    let layout = (content_width > 0).then(|| {
        super::input_bar_wrap::wrap_with_cell_width(&source, content_width as u16, |grapheme| {
            Span::raw(grapheme).width() as u8
        })
    });
    let total_lines = layout.as_ref().map(|layout| layout.rows.len()).unwrap_or(0);
    let max_scroll = if visible_lines == 0 || content_width == 0 {
        0
    } else {
        total_lines.saturating_sub(visible_lines)
    };
    let scroll_offset = scroll_offset.min(max_scroll);

    let title = if visible_lines == 0 || total_lines <= visible_lines {
        "Task detail".to_string()
    } else {
        let start_line = scroll_offset.saturating_add(1);
        let end_line = scroll_offset.saturating_add(visible_lines).min(total_lines);
        let percent = if max_scroll == 0 {
            100
        } else {
            scroll_offset.saturating_mul(100) / max_scroll
        };
        format!("Task detail ({start_line}-{end_line}/{total_lines}) {percent}%")
    };

    let visible: Vec<Line<'static>> = layout
        .into_iter()
        .flat_map(|layout| layout.rows.into_iter())
        .skip(scroll_offset)
        .take(visible_lines)
        .map(|row| {
            let original = &lines[row.logical_row];
            let mut byte = 0;
            let spans = original
                .spans
                .iter()
                .filter_map(|span| {
                    let span_start = byte;
                    byte += span.content.len();
                    let start = row.byte_start.max(span_start);
                    let end = row.byte_end.min(byte);
                    (start < end).then(|| {
                        Span::styled(
                            span.content[start - span_start..end - span_start].to_string(),
                            original.style.patch(span.style),
                        )
                    })
                })
                .collect::<Vec<_>>();
            Line::from(spans).style(original.style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(visible).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
    TaskDetailScroll {
        offset: scroll_offset,
        max_offset: max_scroll,
    }
}

fn token(theme: &Theme, name: &str) -> ratatui::style::Color {
    resolve_token(theme, name, ColorDepth::Truecolor)
}

fn hero_line(task: &TrackedTask, live_node: Option<&ExecutorNode>, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            task.issue_title.as_ref().unwrap_or(&task.task_name).clone(),
            Style::default()
                .fg(token(theme, "plan_inspector.detail.hero.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(
            task.agent.clone(),
            Style::default().fg(token(theme, "plan_inspector.detail.hero.agent.fg")),
        ),
    ];
    if let Some(node) = live_node {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            format!(
                "{} {} {:?}",
                if super::plan_stage_board::is_live_phase(node.phase) {
                    "live"
                } else {
                    "last worker"
                },
                node.agent,
                node.phase
            )
            .to_lowercase(),
            Style::default().fg(token(theme, "plan_inspector.chip.live.fg")),
        ));
    }
    Line::from(spans)
}

fn task_is_blocked(task: &TrackedTask) -> bool {
    matches!(
        task.status.as_str(),
        "blocked" | "blocked_on_setup_conflict"
    ) || !task.blocked_by.is_empty()
}

fn blocked_banner(
    task: &TrackedTask,
    plan: &TrackedPlan,
    content_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let refs = if task.status == "blocked_on_setup_conflict" {
        "setup conflict".to_string()
    } else if task.blocked_by.is_empty() {
        task.status.clone()
    } else {
        task.blocked_by
            .iter()
            .map(|id| task_ref(id, "↑", plan))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut text = format!(" BLOCKED {refs}");
    if content_width > text.len() {
        text.push_str(&" ".repeat(content_width - text.len()));
    }
    Line::from(vec![Span::styled(
        text,
        Style::default()
            .fg(token(theme, "plan_inspector.detail.blocked_banner.fg"))
            .bg(token(theme, "plan_inspector.detail.blocked_banner.bg"))
            .add_modifier(Modifier::BOLD),
    )])
}

fn dependency_strip(
    task: &TrackedTask,
    plan: &TrackedPlan,
    content_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if task.depends_on.is_empty() && task.unblocks.is_empty() && task.blocked_by.is_empty() {
        return Vec::new();
    }

    let parents = dependency_column("Parents", "↑", &task.depends_on, plan);
    let children = dependency_column("Children", "→", &task.unblocks, plan);
    let blocked_by = dependency_column("Blocked by", "↑", &task.blocked_by, plan);

    if content_width < 40 {
        return vec![
            dep_line("Parents", &parents, false, theme),
            dep_line("Children", &children, false, theme),
            dep_line("Blocked by", &blocked_by, true, theme),
        ];
    }

    let col_width = (content_width / 3).max(1);
    vec![Line::from(vec![
        Span::styled(
            pad_column(&format!("Parents {parents}"), col_width),
            Style::default().fg(token(theme, "plan_inspector.detail.edge.structural.fg")),
        ),
        Span::styled(
            pad_column(&format!("Children {children}"), col_width),
            Style::default().fg(token(theme, "plan_inspector.detail.edge.highlight.fg")),
        ),
        Span::styled(
            truncate_to_width(&format!("Blocked by {blocked_by}"), col_width),
            Style::default().fg(token(theme, "plan_inspector.detail.edge.blocked.fg")),
        ),
    ])]
}

fn dependency_column(_label: &str, arrow: &str, ids: &[String], plan: &TrackedPlan) -> String {
    if ids.is_empty() {
        return "--".to_string();
    }
    ids.iter()
        .map(|id| task_ref(id, arrow, plan))
        .collect::<Vec<_>>()
        .join(", ")
}

fn task_ref(id: &str, arrow: &str, plan: &TrackedPlan) -> String {
    match plan.task(id) {
        Some(task) => match status_suffix(&task.status) {
            Some(suffix) => format!("{arrow}{id} {suffix}"),
            None => format!("{arrow}{id}"),
        },
        None => format!("{arrow}{id}"),
    }
}

fn status_suffix(status: &str) -> Option<&'static str> {
    match status {
        "running" | "active" | "dispatched" => Some("(RUN)"),
        "rejected" => Some("(REJ)"),
        "failed" | "error" => Some("(ERR)"),
        _ => None,
    }
}

fn dep_line(label: &'static str, value: &str, blocked: bool, theme: &Theme) -> Line<'static> {
    let token_name = if blocked {
        "plan_inspector.detail.edge.blocked.fg"
    } else {
        "plan_inspector.detail.edge.structural.fg"
    };
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default()
                .fg(token(theme, "plan_inspector.detail.label.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            value.to_string(),
            Style::default().fg(token(theme, token_name)),
        ),
    ])
}

fn execution_strip(task: &TrackedTask, theme: &Theme) -> Line<'static> {
    let mut fields = vec![
        format!("status {}", task.status),
        format!("attempt {}/{}", task.attempt, task.max_attempts),
    ];
    if matches!(task.status.as_str(), "failed" | "rejected" | "error") {
        if task.issue_id.is_none() {
            fields.push("retry unavailable: no issue".into());
        } else if task.attempt >= task.max_attempts {
            fields.push("retry exhausted".into());
        } else {
            fields.push("R retry".into());
        }
    }
    if let Some(branch) = task.worker_branch.as_deref() {
        fields.push(format!("branch {branch}"));
    }
    if let Some(delegation_id) = task.delegation_id.as_deref() {
        fields.push(format!("delegation {delegation_id}"));
    }
    Line::from(vec![Span::styled(
        fields.join(" · "),
        Style::default().fg(token(theme, "plan_inspector.detail.section.fg")),
    )])
}

fn pad_column(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    let len = truncated.chars().count();
    if len >= width {
        truncated
    } else {
        format!("{truncated}{}", " ".repeat(width - len))
    }
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }

    let mut truncated: String = value.chars().take(width - 1).collect();
    truncated.push('…');
    truncated
}

fn section_header(title: &str, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::raw(""),
        Line::from(vec![Span::styled(
            title.to_string(),
            Style::default()
                .fg(token(theme, "plan_inspector.detail.label.fg"))
                .add_modifier(Modifier::BOLD),
        )]),
    ]
}

fn labels_block(labels: &[String], theme: &Theme) -> Vec<Line<'static>> {
    if labels.is_empty() {
        return Vec::new();
    }
    if labels.len() == 1 {
        return vec![kv(theme, "labels", &labels[0])];
    }
    let mut lines = vec![Line::from(vec![Span::styled(
        "labels:".to_string(),
        Style::default()
            .fg(token(theme, "plan_inspector.detail.label.fg"))
            .add_modifier(Modifier::BOLD),
    )])];
    for label in labels {
        lines.push(Line::from(vec![
            Span::styled(
                "  · ",
                Style::default().fg(token(theme, "plan_inspector.detail.section.fg")),
            ),
            Span::raw(label.clone()),
        ]));
    }
    lines
}

fn format_date_with_age(when: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let date = when.format("%Y-%m-%d");
    let secs = (now - when).num_seconds();
    let age = relative_age(secs);
    match age {
        Some(label) => format!("{date} ({label})"),
        None => date.to_string(),
    }
}

fn relative_age(secs: i64) -> Option<String> {
    if secs.abs() < 60 {
        return Some("just now".to_string());
    }
    let (magnitude, suffix) = if secs >= 0 {
        (secs, "ago")
    } else {
        (-secs, "from now")
    };
    let label = if magnitude < 60 * 60 {
        format!("{}m", magnitude / 60)
    } else if magnitude < 24 * 60 * 60 {
        format!("{}h", magnitude / 3600)
    } else if magnitude < 30 * 24 * 60 * 60 {
        format!("{}d", magnitude / 86_400)
    } else if magnitude < 365 * 24 * 60 * 60 {
        format!("{}mo", magnitude / (30 * 86_400))
    } else {
        format!("{}y", magnitude / (365 * 86_400))
    };
    Some(format!("{label} {suffix}"))
}

fn kv(theme: &Theme, label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default()
                .fg(token(theme, "plan_inspector.detail.label.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(value.to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, widgets::Paragraph, Terminal};
    use spur_acp::{PlanSnapshotCounts, SessionId};
    use spur_core::{TrackedPlan, TrackedTask};

    use super::dependency_strip;
    use crate::theme::{load_built_in, Theme};

    fn task(task_id: &str) -> TrackedTask {
        TrackedTask {
            task_id: task_id.into(),
            task_name: format!("{task_id} task"),
            agent: "codex".into(),
            issue_id: Some(format!("bd-1dwm.{task_id}")),
            issue_title: None,
            status: "pending".into(),
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

    fn render_dependency_strip(task: &TrackedTask, plan: &TrackedPlan, theme: &Theme) -> String {
        let content_width = 54;
        let backend = TestBackend::new(content_width as u16, 1);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let lines = dependency_strip(task, plan, content_width, theme);

        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(lines), frame.area()))
            .expect("render dependency strip");

        rendered_buffer_text(&terminal)
    }

    #[test]
    fn dependency_strip_marks_truncated_columns_with_ellipsis() {
        let theme = load_built_in("dark").expect("built-in dark theme");

        let mut long = task("t-main");
        long.depends_on = vec!["t-901".into(), "t-13".into(), "t-42".into()];
        long.blocked_by = vec!["t-777".into(), "t-888".into(), "t-999".into()];
        let long_plan = plan_with_tasks(vec![long.clone()]);

        let rendered = render_dependency_strip(&long, &long_plan, &theme);
        assert!(
            rendered.contains('…'),
            "expected visible truncation indicator; rendered: {rendered}"
        );
        assert!(
            rendered.contains("Children --"),
            "short child column should render unchanged; rendered: {rendered}"
        );

        let mut short = task("t-main");
        short.depends_on = vec!["t-1".into()];
        short.blocked_by = vec!["t-2".into()];
        let short_plan = plan_with_tasks(vec![short.clone()]);

        let rendered = render_dependency_strip(&short, &short_plan, &theme);
        assert!(rendered.contains("Parents ↑t-1"), "rendered: {rendered}");
        assert!(rendered.contains("Blocked by ↑t-2"), "rendered: {rendered}");
        assert!(
            !rendered.contains('…'),
            "short values should not be marked as truncated; rendered: {rendered}"
        );
    }
}
