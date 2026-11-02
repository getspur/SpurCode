use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use spur_core::{TrackedPlan, TrackedTask};
use unicode_segmentation::UnicodeSegmentation;

use super::{display_cells, fit_to_cells, token, PlanInspectorView};
use crate::{
    action::Action, components::plan_stage_board::stage_grouped_tasks, views::ViewContext,
};

pub(super) struct TaskSearch {
    query: String,
    selected_task_id: Option<String>,
}

impl TaskSearch {
    fn matches<'a>(&self, plan: &'a TrackedPlan) -> Vec<&'a TrackedTask> {
        let query = self.query.trim().to_lowercase();
        stage_grouped_tasks(plan)
            .into_iter()
            .filter(|task| {
                query.is_empty()
                    || task.task_name.to_lowercase().contains(&query)
                    || task.task_id.to_lowercase().contains(&query)
                    || task
                        .issue_id
                        .as_deref()
                        .is_some_and(|id| id.to_lowercase().contains(&query))
            })
            .collect()
    }

    fn reconcile(&mut self, matches: &[&TrackedTask]) {
        if !self
            .selected_task_id
            .as_ref()
            .is_some_and(|id| matches.iter().any(|task| &task.task_id == id))
        {
            self.selected_task_id = matches.first().map(|task| task.task_id.clone());
        }
    }
}

impl PlanInspectorView {
    pub(super) fn open_task_search(&mut self) {
        self.search = Some(TaskSearch {
            query: String::new(),
            selected_task_id: self.selected_task_id.clone(),
        });
    }

    pub(super) fn handle_task_search_key(
        &mut self,
        key: KeyEvent,
        ctx: &ViewContext<'_>,
    ) -> Option<Action> {
        if key.code == KeyCode::Esc {
            self.search = None;
            return None;
        }
        let plan = self.active_plan(ctx);
        let search = self.search.as_mut()?;
        let matches = plan.map(|plan| search.matches(plan)).unwrap_or_default();
        search.reconcile(&matches);
        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(target) = search.selected_task_id.clone() else {
                    return Some(Action::FlashHint {
                        message: "No matching tasks".into(),
                    });
                };
                self.search = None;
                self.set_selected_task_id(Some(target));
                self.detail_focused = false;
                return None;
            }
            KeyCode::Up | KeyCode::Down => {
                let index = matches
                    .iter()
                    .position(|task| Some(&task.task_id) == search.selected_task_id.as_ref())
                    .unwrap_or(0);
                let next = if key.code == KeyCode::Up {
                    index.saturating_sub(1)
                } else {
                    index.saturating_add(1).min(matches.len().saturating_sub(1))
                };
                search.selected_task_id = matches.get(next).map(|task| task.task_id.clone());
            }
            KeyCode::Backspace if key.modifiers.is_empty() => {
                if let Some((start, _)) = search.query.grapheme_indices(true).next_back() {
                    search.query.truncate(start);
                }
                search.selected_task_id = None;
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                search.query.push(ch);
                search.selected_task_id = None;
            }
            _ => {}
        }
        let matches = plan.map(|plan| search.matches(plan)).unwrap_or_default();
        search.reconcile(&matches);
        None
    }

    pub(super) fn render_task_search(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        ctx: &ViewContext<'_>,
    ) {
        let plan = self.active_plan(ctx);
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let matches = plan.map(|plan| search.matches(plan)).unwrap_or_default();
        search.reconcile(&matches);
        let width = area.width.saturating_sub(2) as usize;
        // Borders consume two rows; the query consumes one content row.
        let result_rows = area.height.saturating_sub(3) as usize;
        // Prefix and one cursor cell share the query row with its visible tail.
        let query = query_tail(&search.query, width.saturating_sub(display_cells("/ ") + 1));
        let mut lines = vec![Line::from(format!("/ {query}"))];
        let selected = matches
            .iter()
            .position(|task| Some(&task.task_id) == search.selected_task_id.as_ref())
            .unwrap_or(0);
        let offset = selected.saturating_sub(result_rows.saturating_sub(1));
        if matches.is_empty() {
            lines.push(Line::from("No matching tasks"));
        } else {
            for task in matches.iter().skip(offset).take(result_rows) {
                let selected = Some(&task.task_id) == search.selected_task_id.as_ref();
                let issue = task
                    .issue_id
                    .as_ref()
                    .map(|id| format!(" [{id}]"))
                    .unwrap_or_default();
                let text = format!(
                    "{}{}{}  {}",
                    if selected { "▶ " } else { "  " },
                    task.task_id,
                    issue,
                    task.task_name
                );
                lines.push(Line::from(Span::styled(
                    fit_to_cells(&text, width),
                    if selected {
                        Style::default()
                            .fg(token(ctx.theme, "plan_inspector.board.selection.fg"))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                )));
            }
        }
        let title = fit_to_cells(&format!("Find task ({} matches)", matches.len()), width);
        let footer = fit_to_cells("Esc cancel  Enter select  ↑/↓ results", width);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_bottom(footer)
                    .style(
                        Style::default()
                            .fg(ctx.theme.palette.fg.rgb)
                            .bg(ctx.theme.palette.bg.rgb),
                    )
                    .border_style(
                        Style::default()
                            .fg(token(ctx.theme, "plan_inspector.board.stage.active.fg")),
                    ),
            ),
            area,
        );
        if area.width > 3 && area.height > 2 {
            let x = area
                .x
                .saturating_add(1 + display_cells("/ ") as u16 + display_cells(&query) as u16)
                .min(area.x.saturating_add(area.width.saturating_sub(2)));
            frame.set_cursor_position((x, area.y.saturating_add(1)));
        }
    }

    pub(super) fn jump_to_attention(
        &mut self,
        plan: &TrackedPlan,
        forward: bool,
    ) -> Option<Action> {
        let ordered = stage_grouped_tasks(plan);
        let current = ordered
            .iter()
            .position(|task| Some(&task.task_id) == self.selected_task_id.as_ref())
            .unwrap_or(0);
        let target = if forward {
            ordered
                .iter()
                .skip(current.saturating_add(1))
                .chain(ordered.iter().take(current.saturating_add(1)))
                .find(|task| needs_attention(task))
        } else {
            ordered
                .iter()
                .take(current)
                .rev()
                .chain(ordered.iter().skip(current).rev())
                .find(|task| needs_attention(task))
        };
        if let Some(task) = target {
            self.set_selected_task_id(Some(task.task_id.clone()));
            self.detail_focused = false;
            None
        } else {
            Some(Action::FlashHint {
                message: "No tasks need attention".into(),
            })
        }
    }
}

fn needs_attention(task: &TrackedTask) -> bool {
    !matches!(
        task.status.as_str(),
        "approved" | "cancelled" | "superseded"
    ) && (!task.blocked_by.is_empty()
        || matches!(
            task.status.as_str(),
            "awaiting_review"
                | "escalated_to_brain"
                | "failed"
                | "error"
                | "rejected"
                | "blocked_on_setup_conflict"
        ))
}

fn query_tail(value: &str, width: usize) -> String {
    if display_cells(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    let mut tail = Vec::new();
    for grapheme in value.graphemes(true).rev() {
        let cells = display_cells(grapheme);
        if used + cells > width - 1 {
            break;
        }
        used += cells;
        tail.push(grapheme);
    }
    tail.reverse();
    format!("…{}", tail.concat())
}
