use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};
use spur_acp::{SessionId, SpurEvent};
use spur_core::{ExecutorId, ExecutorNode, LifecycleState, TrackedPlan};

use crate::action::{Action, IssueAction};
use crate::theme::{resolve_token, ColorDepth, Theme};

use super::plan_provenance::loop_origin_badge;
use super::View;

#[derive(Debug)]
pub enum PlanInspectorMode {
    Browse,
    StreamPeek {
        executor_id: String,
        task_id: String,
        state: crate::components::stream_pane::StreamViewState,
    },
}

fn token(theme: &Theme, name: &str) -> Color {
    resolve_token(theme, name, ColorDepth::Truecolor)
}

pub struct PlanInspectorView {
    session_id: SessionId,
    pinned_plan_id: Option<String>,
    selected_task_id: Option<String>,
    blocker_cycle: Option<(String, usize)>,
    stacked_mode: bool,
    open_issue_id: Option<String>,
    issue_states: HashMap<String, TaskIssueState>,
    task_detail_scroll: usize,
    task_detail_max_scroll: usize,
    task_detail_to_bottom: bool,
    detail_focused: bool,
    mode: PlanInspectorMode,
    confirm: Option<PlanInspectorConfirm>,
    loop_event_status: Option<PlanInspectorEventStatus>,
}

#[derive(Debug)]
enum TaskIssueState {
    Loading,
    Loaded(Box<spur_pm::Issue>),
    Error(String),
}

#[derive(Debug)]
enum PlanInspectorEventStatus {
    CommandError(String),
    Loop(String),
}

impl PlanInspectorEventStatus {
    fn message(&self) -> &str {
        match self {
            Self::CommandError(message) | Self::Loop(message) => message,
        }
    }
}

#[derive(Debug, Clone)]
enum PlanInspectorConfirm {
    Start {
        plan_id: String,
    },
    RetryTask {
        plan_id: String,
        task_id: String,
        task_name: String,
        issue_id: String,
        status: String,
        attempt: u32,
        max_attempts: u32,
        append_prompt: String,
        cursor: usize,
    },
    StopTask {
        plan_id: String,
        task_id: String,
        task_name: String,
        delegation_id: String,
        status: String,
        retry_guidance: String,
    },
    Review {
        plan_id: String,
        task_id: String,
        task_name: String,
        status: String,
        executor_id: String,
        attempt_n: u32,
        decision: spur_core::ReviewDecision,
        feedback: Option<String>,
    },
}

impl PlanInspectorView {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            pinned_plan_id: None,
            selected_task_id: None,
            blocker_cycle: None,
            stacked_mode: false,
            open_issue_id: None,
            issue_states: HashMap::new(),
            task_detail_scroll: 0,
            task_detail_max_scroll: 0,
            task_detail_to_bottom: false,
            detail_focused: false,
            mode: PlanInspectorMode::Browse,
            confirm: None,
            loop_event_status: None,
        }
    }

    pub fn new_for_plan(session_id: SessionId, plan_id: String) -> Self {
        Self {
            session_id,
            pinned_plan_id: Some(plan_id),
            selected_task_id: None,
            blocker_cycle: None,
            stacked_mode: false,
            open_issue_id: None,
            issue_states: HashMap::new(),
            task_detail_scroll: 0,
            task_detail_max_scroll: 0,
            task_detail_to_bottom: false,
            detail_focused: false,
            mode: PlanInspectorMode::Browse,
            confirm: None,
            loop_event_status: None,
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn mode(&self) -> &PlanInspectorMode {
        &self.mode
    }

    pub fn enter_stream_peek(&mut self, executor_id: String, task_id: String) {
        self.mode = PlanInspectorMode::StreamPeek {
            executor_id,
            task_id,
            state: crate::components::stream_pane::StreamViewState::new(),
        };
    }

    pub fn leave_stream_peek(&mut self) {
        self.mode = PlanInspectorMode::Browse;
    }

    #[allow(dead_code)]
    fn peek_state_mut(&mut self) -> Option<&mut crate::components::stream_pane::StreamViewState> {
        if let PlanInspectorMode::StreamPeek { state, .. } = &mut self.mode {
            Some(state)
        } else {
            None
        }
    }

    #[cfg(test)]
    fn set_selected_task_id_for_tests(&mut self, task_id: Option<String>) {
        self.set_selected_task_id(task_id);
    }

    #[cfg(test)]
    fn set_open_issue_id_for_tests(&mut self, id: Option<String>) {
        self.open_issue_id = id;
    }

    #[cfg(test)]
    fn selected_task_id_for_tests(&self) -> Option<String> {
        self.selected_task_id.clone()
    }

    #[cfg(test)]
    fn peek_state_mut_for_tests(
        &mut self,
    ) -> Option<&mut crate::components::stream_pane::StreamViewState> {
        self.peek_state_mut()
    }

    fn active_plan<'a>(&self, ctx: &'a super::ViewContext<'_>) -> Option<&'a TrackedPlan> {
        match self.pinned_plan_id.as_deref() {
            Some(plan_id) => ctx.plan_projection.plan(plan_id),
            None => ctx.plan_projection.current_for_session(&self.session_id),
        }
    }

    fn set_selected_task_id(&mut self, task_id: Option<String>) {
        self.blocker_cycle = None;
        self.set_selected_task_id_inner(task_id);
    }

    fn set_selected_task_id_from_blocker_jump(&mut self, task_id: Option<String>) {
        self.set_selected_task_id_inner(task_id);
    }

    fn set_selected_task_id_inner(&mut self, task_id: Option<String>) {
        let previous_task_id = self.selected_task_id.as_deref();
        let selection_changed = previous_task_id != task_id.as_deref();
        if selection_changed {
            self.open_issue_id = None;
            self.task_detail_scroll = 0;
            self.task_detail_max_scroll = 0;
            self.task_detail_to_bottom = false;
            if previous_task_id.is_some()
                && matches!(self.mode, PlanInspectorMode::StreamPeek { .. })
            {
                self.mode = PlanInspectorMode::Browse;
            }
        }
        self.selected_task_id = task_id;
    }

    fn close_issue_detail(&mut self) {
        self.open_issue_id = None;
        self.task_detail_scroll = 0;
        self.task_detail_to_bottom = false;
    }

    fn scroll_task_detail_up(&mut self, lines: usize) {
        self.task_detail_to_bottom = false;
        self.task_detail_scroll = self
            .task_detail_scroll
            .min(self.task_detail_max_scroll)
            .saturating_sub(lines);
    }

    fn scroll_task_detail_down(&mut self, lines: usize) {
        self.task_detail_to_bottom = false;
        self.task_detail_scroll = self
            .task_detail_scroll
            .saturating_add(lines)
            .min(self.task_detail_max_scroll);
    }

    fn scroll_task_detail_to_top(&mut self) {
        self.task_detail_scroll = 0;
        self.task_detail_to_bottom = false;
    }

    fn scroll_task_detail_to_bottom(&mut self) {
        self.task_detail_scroll = self.task_detail_max_scroll;
        self.task_detail_to_bottom = true;
    }

    fn selected_issue_id<'a>(&self, task: &'a spur_core::TrackedTask) -> Option<&'a str> {
        task.issue_id.as_deref()
    }

    fn toggle_issue_detail(&mut self, task: &spur_core::TrackedTask) -> Option<Action> {
        let issue_id = self.selected_issue_id(task)?;
        if self.open_issue_id.as_deref() == Some(issue_id) {
            self.close_issue_detail();
            return None;
        }

        self.open_issue_id = Some(issue_id.to_string());
        self.task_detail_scroll = 0;
        let needs_request = !matches!(
            self.issue_states.get(issue_id),
            Some(TaskIssueState::Loaded(_))
        );
        if needs_request {
            self.issue_states
                .insert(issue_id.to_string(), TaskIssueState::Loading);
            Some(Action::Issue(IssueAction::ViewDetail {
                id: issue_id.to_string(),
            }))
        } else {
            None
        }
    }

    fn issue_detail_for_selected(
        &self,
        task: &spur_core::TrackedTask,
    ) -> (Option<&spur_pm::Issue>, Option<&str>) {
        let issue_id = match self.selected_issue_id(task) {
            Some(id) => id,
            None => return (None, None),
        };

        if self.open_issue_id.as_deref() != Some(issue_id) {
            return (None, None);
        }

        match self.issue_states.get(issue_id) {
            Some(TaskIssueState::Loaded(issue)) => (Some(issue.as_ref()), None),
            Some(TaskIssueState::Loading) => (None, Some("Loading issue detail...")),
            Some(TaskIssueState::Error(error)) => (None, Some(error.as_str())),
            None => (None, None),
        }
    }

    fn detail_event_to_issue(event: &spur_acp::IssueDetailEvent) -> spur_pm::Issue {
        spur_pm::Issue {
            id: event.id.clone(),
            source: match event.source.as_str() {
                "github" => spur_pm::PmSource::GitHub,
                "linear" => spur_pm::PmSource::Linear,
                "plane" => spur_pm::PmSource::Plane,
                _ => spur_pm::PmSource::Beads,
            },
            title: event.title.clone(),
            status: event.status.clone(),
            priority: event.priority,
            issue_type: event.issue_type.clone(),
            assignee: event.assignee.clone(),
            due_at: event.due_at,
            blocked_by: event.blocked_by.clone(),
            labels: event.labels.clone(),
            url: event.url.clone(),
            body: event.body.clone(),
            created_at: event.created_at,
            updated_at: event.updated_at,
            external_ref: None,
            source_system: None,
            source_repo: None,
        }
    }

    fn ensure_selection(&mut self, plan: &TrackedPlan) {
        let selected_exists = self
            .selected_task_id
            .as_ref()
            .and_then(|task_id| plan.task(task_id))
            .is_some();
        if selected_exists {
            return;
        }
        self.set_selected_task_id(
            crate::components::plan_stage_board::stage_grouped_tasks(plan)
                .first()
                .map(|task| task.task_id.clone()),
        );
    }

    fn selected_task<'a>(&self, plan: &'a TrackedPlan) -> Option<&'a spur_core::TrackedTask> {
        self.selected_task_id
            .as_ref()
            .and_then(|task_id| plan.task(task_id))
    }

    fn current_stage(&self, plan: &TrackedPlan) -> usize {
        self.selected_task(plan)
            .map(|task| task.stage_idx)
            .unwrap_or(0)
    }

    fn tasks_in_stage(plan: &TrackedPlan, stage_idx: usize) -> Vec<&spur_core::TrackedTask> {
        plan.tasks
            .iter()
            .filter(|task| task.stage_idx == stage_idx)
            .collect()
    }

    fn max_stage(plan: &TrackedPlan) -> usize {
        plan.tasks
            .iter()
            .map(|task| task.stage_idx)
            .max()
            .unwrap_or(0)
    }

    fn select_first_in_stage(&mut self, plan: &TrackedPlan, stage_idx: usize) {
        self.set_selected_task_id(
            Self::tasks_in_stage(plan, stage_idx)
                .first()
                .map(|task| task.task_id.clone()),
        );
    }

    fn move_lane(&mut self, plan: &TrackedPlan, delta: isize) {
        self.close_issue_detail();
        let current = self.current_stage(plan) as isize;
        let next = (current + delta).clamp(0, Self::max_stage(plan) as isize) as usize;
        self.select_first_in_stage(plan, next);
    }

    fn move_task(&mut self, plan: &TrackedPlan, delta: isize) {
        self.close_issue_detail();
        let tasks: Vec<_> = if self.stacked_mode {
            crate::components::plan_stage_board::stage_grouped_tasks(plan)
        } else {
            let stage_idx = self.current_stage(plan);
            Self::tasks_in_stage(plan, stage_idx)
        };
        if tasks.is_empty() {
            return;
        }
        let current_idx = self
            .selected_task(plan)
            .and_then(|task| {
                tasks
                    .iter()
                    .position(|candidate| candidate.task_id == task.task_id)
            })
            .unwrap_or(0) as isize;
        let next = (current_idx + delta).clamp(0, tasks.len() as isize - 1) as usize;
        self.set_selected_task_id(Some(tasks[next].task_id.clone()));
    }

    fn jump_lane_start(&mut self, plan: &TrackedPlan) {
        self.select_first_in_stage(plan, self.current_stage(plan));
    }

    fn jump_lane_end(&mut self, plan: &TrackedPlan) {
        let stage_idx = self.current_stage(plan);
        if let Some(task) = Self::tasks_in_stage(plan, stage_idx).last() {
            self.set_selected_task_id(Some(task.task_id.clone()));
        }
    }

    fn jump_to_next_blocker(&mut self, plan: &TrackedPlan) -> Option<Action> {
        let selected_id = self.selected_task_id.as_deref()?;
        let origin_id = self
            .blocker_cycle
            .as_ref()
            .and_then(|(origin_id, _)| {
                let origin = plan.task(origin_id)?;
                let selected_is_origin = selected_id == origin_id;
                let selected_is_blocker = origin
                    .blocked_by
                    .iter()
                    .any(|blocker_id| blocker_id == selected_id);
                (selected_is_origin || selected_is_blocker).then(|| origin_id.clone())
            })
            .unwrap_or_else(|| selected_id.to_string());
        let origin = match plan.task(&origin_id) {
            Some(task) => task,
            None => {
                self.blocker_cycle = None;
                return Some(Action::FlashHint {
                    message: "Selected task is no longer in the current plan".into(),
                });
            }
        };
        if origin.blocked_by.is_empty() {
            self.blocker_cycle = None;
            return Some(Action::FlashHint {
                message: "Selected task has no blockers".into(),
            });
        }

        let next_idx = self
            .blocker_cycle
            .as_ref()
            .filter(|(cycle_origin, _)| cycle_origin == &origin_id)
            .map(|(_, idx)| *idx)
            .unwrap_or(0);
        let blocker_id = origin.blocked_by[next_idx % origin.blocked_by.len()].clone();
        self.blocker_cycle = Some((origin_id, next_idx + 1));
        if plan.task(&blocker_id).is_some() {
            self.set_selected_task_id_from_blocker_jump(Some(blocker_id));
            None
        } else {
            Some(Action::FlashHint {
                message: format!("Blocker {blocker_id} is not in the current plan"),
            })
        }
    }

    fn start_or_resume_plan(&mut self, plan: &TrackedPlan) -> Option<Action> {
        if self.open_issue_id.is_some() || !matches!(self.mode, PlanInspectorMode::Browse) {
            return None;
        }

        if !plan.is_active() {
            return Some(Action::FlashHint {
                message: format!("Cannot start: plan {} is terminal", plan.plan_id),
            });
        }

        self.confirm = Some(PlanInspectorConfirm::Start {
            plan_id: plan.plan_id.clone(),
        });
        None
    }

    fn retry_selected_task(&mut self, plan: &TrackedPlan) -> Option<Action> {
        if self.open_issue_id.is_some() || !matches!(self.mode, PlanInspectorMode::Browse) {
            return None;
        }

        let Some(task) = self.selected_task(plan) else {
            return Some(Action::FlashHint {
                message: "Cannot retry: no selected task".into(),
            });
        };
        let Some(issue_id) = task.issue_id.clone() else {
            return Some(Action::FlashHint {
                message: "Cannot retry: selected task has no linked issue".into(),
            });
        };

        if !is_retryable_task_status(&task.status) {
            return Some(Action::FlashHint {
                message: format!("Cannot retry: task {} is {}", task.task_id, task.status),
            });
        }

        if task.attempt >= task.max_attempts {
            return Some(Action::FlashHint {
                message: format!(
                    "Cannot retry: task {} is at attempt {}/{}",
                    task.task_id, task.attempt, task.max_attempts
                ),
            });
        }

        self.confirm = Some(PlanInspectorConfirm::RetryTask {
            plan_id: plan.plan_id.clone(),
            task_id: task.task_id.clone(),
            task_name: task.task_name.clone(),
            issue_id,
            status: task.status.clone(),
            attempt: task.attempt,
            max_attempts: task.max_attempts,
            append_prompt: String::new(),
            cursor: 0,
        });
        None
    }

    fn stop_selected_task(&mut self, plan: &TrackedPlan) -> Option<Action> {
        if self.open_issue_id.is_some() || !matches!(self.mode, PlanInspectorMode::Browse) {
            return None;
        }

        let Some(task) = self.selected_task(plan) else {
            return Some(Action::FlashHint {
                message: "Cannot stop: no selected task".into(),
            });
        };

        if !is_stoppable_task_status(&task.status) {
            return Some(Action::FlashHint {
                message: format!("Cannot stop: task {} is {}", task.task_id, task.status),
            });
        }

        let Some(delegation_id) = task.delegation_id.clone() else {
            return Some(Action::FlashHint {
                message: format!("Cannot stop: task {} has no delegation", task.task_id),
            });
        };

        self.confirm = Some(PlanInspectorConfirm::StopTask {
            plan_id: plan.plan_id.clone(),
            task_id: task.task_id.clone(),
            task_name: task.task_name.clone(),
            delegation_id,
            status: task.status.clone(),
            retry_guidance: retry_guidance_after_stop(task),
        });
        None
    }

    fn review_selected_task(
        &mut self,
        plan: &TrackedPlan,
        lineage: &spur_core::ExecutorLineage,
        key: char,
    ) -> Option<Action> {
        if self.open_issue_id.is_some() || !matches!(self.mode, PlanInspectorMode::Browse) {
            return None;
        }

        let Some(task) = self.selected_task(plan) else {
            return Some(Action::FlashHint {
                message: "Cannot review: no selected task".into(),
            });
        };

        if !is_reviewable_task_status(&task.status) {
            return Some(Action::FlashHint {
                message: format!("Cannot review: task {} is {}", task.task_id, task.status),
            });
        }

        let Some(delegation_id) = task.delegation_id.as_ref() else {
            return Some(Action::FlashHint {
                message: format!("Cannot review: task {} has no delegation", task.task_id),
            });
        };
        let Some(executor_id) = lineage
            .executor_id_for_delegation(&spur_acp::domain::delegation::DelegationId(
                delegation_id.clone(),
            ))
            .map(|id| id.0.clone())
        else {
            return Some(Action::FlashHint {
                message: format!("Cannot review: task {} has no linked worker", task.task_id),
            });
        };
        let Some(pending_review) = lineage
            .node(&ExecutorId(executor_id.clone()))
            .and_then(|node| node.pending_review.as_ref())
        else {
            return Some(Action::FlashHint {
                message: format!("Cannot review: task {} has no pending review", task.task_id),
            });
        };

        let feedback = task.feedback.clone();
        let decision = plan_review_decision_for_key(key, feedback.clone())?;

        self.confirm = Some(PlanInspectorConfirm::Review {
            plan_id: plan.plan_id.clone(),
            task_id: task.task_id.clone(),
            task_name: task.task_name.clone(),
            status: task.status.clone(),
            executor_id,
            attempt_n: pending_review.attempt_n,
            decision,
            feedback,
        });
        None
    }

    fn confirm_action(&mut self) -> Option<Action> {
        let confirm = self.confirm.take()?;
        match confirm {
            PlanInspectorConfirm::Start { plan_id } => Some(Action::ResumePlan { plan_id }),
            PlanInspectorConfirm::RetryTask {
                plan_id,
                issue_id,
                append_prompt,
                ..
            } => {
                if matches!(
                    self.loop_event_status.as_ref(),
                    Some(PlanInspectorEventStatus::CommandError(_))
                ) {
                    self.loop_event_status = None;
                }
                Some(Action::RetryPlanTask {
                    plan_id: Some(plan_id),
                    issue_id,
                    append_prompt: non_empty_prompt(append_prompt),
                })
            }
            PlanInspectorConfirm::StopTask { delegation_id, .. } => {
                Some(Action::CancelDelegation { delegation_id })
            }
            PlanInspectorConfirm::Review {
                executor_id,
                attempt_n,
                decision,
                ..
            } => Some(Action::SubmitReview {
                executor_id,
                attempt_n,
                decision,
            }),
        }
    }
}

impl PlanInspectorView {
    pub fn render_with_worker_streams(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        worker_streams: &mut crate::worker_streams::WorkerStreams,
        ctx: &super::ViewContext,
    ) {
        <Self as View>::render(self, frame, area, ctx);

        if self.stream_peek_executor_is_stale(ctx) {
            self.leave_stream_peek();
            return;
        }

        if let PlanInspectorMode::StreamPeek {
            executor_id,
            task_id,
            state,
        } = &mut self.mode
        {
            let trace = worker_streams.get_mut(executor_id);
            let node = ctx.lineage.node(&ExecutorId(executor_id.clone()));
            Self::render_peek_overlay(
                frame,
                area,
                executor_id,
                task_id,
                node,
                trace,
                state,
                ctx.theme,
            );
        }
    }

    fn stream_peek_executor_is_stale(&self, ctx: &super::ViewContext) -> bool {
        let PlanInspectorMode::StreamPeek {
            executor_id,
            task_id,
            ..
        } = &self.mode
        else {
            return false;
        };

        let Some(task) = self
            .active_plan(ctx)
            .and_then(|plan| plan.task(task_id.as_str()))
        else {
            return true;
        };

        let current_executor_id = task.delegation_id.as_ref().and_then(|delegation_id| {
            ctx.lineage
                .executor_id_for_delegation(&spur_acp::domain::delegation::DelegationId(
                    delegation_id.clone(),
                ))
        });

        current_executor_id
            .map(|current| current.0 != *executor_id)
            .unwrap_or(true)
    }

    pub fn handle_key_with_worker_streams(
        &mut self,
        key: KeyEvent,
        worker_streams: &mut crate::worker_streams::WorkerStreams,
        ctx: &super::ViewContext,
    ) -> Option<Action> {
        let _ = worker_streams;

        if let PlanInspectorMode::StreamPeek { .. } = &self.mode {
            if let Some(action) = self.handle_peek_key(key) {
                return Some(action);
            }
            return None;
        }

        if let Some(action) = self.maybe_handle_open_peek(key, ctx) {
            return Some(action);
        }

        <Self as View>::handle_key(self, key, ctx)
    }

    fn handle_prompt_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.confirm_action(),
            KeyCode::Esc if key.modifiers.is_empty() => {
                self.confirm = None;
                None
            }
            KeyCode::Backspace if key.modifiers.is_empty() => {
                if let Some((append_prompt, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    delete_before_cursor(append_prompt, cursor);
                }
                None
            }
            KeyCode::Left if key.modifiers.is_empty() => {
                if let Some((append_prompt, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    *cursor = previous_char_boundary(append_prompt, *cursor);
                }
                None
            }
            KeyCode::Right if key.modifiers.is_empty() => {
                if let Some((append_prompt, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    *cursor = next_char_boundary(append_prompt, *cursor);
                }
                None
            }
            KeyCode::Home if key.modifiers.is_empty() => {
                if let Some((_, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    *cursor = 0;
                }
                None
            }
            KeyCode::End if key.modifiers.is_empty() => {
                if let Some((append_prompt, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    *cursor = append_prompt.len();
                }
                None
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some((append_prompt, cursor)) = confirm_prompt_mut(&mut self.confirm) {
                    insert_at_cursor(append_prompt, cursor, ch);
                }
                None
            }
            _ => None,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "overlay rendering keeps borrowed frame and view state explicit"
    )]
    fn render_peek_overlay(
        frame: &mut Frame,
        parent_area: Rect,
        executor_id: &str,
        task_id: &str,
        node: Option<&ExecutorNode>,
        trace: Option<&mut crate::components::react_trace::ReactTrace>,
        state: &mut crate::components::stream_pane::StreamViewState,
        theme: &Theme,
    ) {
        let area = if parent_area.width >= 60 {
            let width =
                ((parent_area.width as u32 * 80 / 100).max(60) as u16).min(parent_area.width);
            let height =
                ((parent_area.height as u32 * 60 / 100).max(8) as u16).min(parent_area.height);
            let x = parent_area.x + parent_area.width.saturating_sub(width) / 2;
            let y = parent_area.y + parent_area.height.saturating_sub(height) / 2;
            Rect::new(x, y, width, height)
        } else {
            parent_area
        };

        frame.render_widget(Clear, area);
        let popup_block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(token(theme, "plan_browser.confirm.border.fg")));
        let stream_area = popup_block.inner(area);
        frame.render_widget(popup_block, area);
        if stream_area.width == 0 || stream_area.height == 0 {
            return;
        }

        let title_name = node
            .map(|node| node.agent.as_str())
            .filter(|agent| !agent.is_empty())
            .unwrap_or(executor_id);
        let title_left = format!("stream: {title_name} ({task_id})");
        let title_right =
            node.and_then(|node| is_terminal_phase(node.phase).then_some("[completed]"));
        let bottom_hint = "[esc] close · [j/k] scroll · [f] follow";

        crate::components::stream_pane::render_stream(
            frame,
            stream_area,
            &title_left,
            title_right,
            Some(bottom_hint),
            trace,
            state,
        );
    }

    fn handle_peek_key(&mut self, key: KeyEvent) -> Option<Action> {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.leave_stream_peek();
                None
            }
            (KeyCode::Char('S'), m) if m == KeyModifiers::SHIFT || m == KeyModifiers::NONE => {
                let executor_id = match &self.mode {
                    PlanInspectorMode::StreamPeek { executor_id, .. } => executor_id.clone(),
                    _ => return None,
                };
                self.leave_stream_peek();
                Some(Action::FocusWorkerInDashboard {
                    executor_id,
                    tab: crate::components::detail_pane::DetailTab::Stream,
                })
            }
            _ => {
                let state = self.peek_state_mut()?;
                match (key.code, key.modifiers) {
                    (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                        state.scroll_down_by(1);
                        None
                    }
                    (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                        state.scroll_up_by(1);
                        None
                    }
                    (KeyCode::Char('g'), KeyModifiers::NONE) => {
                        state.scroll_to_top();
                        None
                    }
                    (KeyCode::Char('G'), _) => {
                        state.scroll_to_bottom();
                        None
                    }
                    (KeyCode::Char('f'), KeyModifiers::NONE) => {
                        state.toggle_follow();
                        None
                    }
                    _ => None,
                }
            }
        }
    }

    fn maybe_handle_open_peek(
        &mut self,
        key: KeyEvent,
        ctx: &super::ViewContext,
    ) -> Option<Action> {
        if self.open_issue_id.is_some() {
            // The issue detail overlay owns modal focus; ignore peek shortcuts here
            // and let the base view consume or ignore the key without stacking modals.
            return None;
        }

        let plan = self.active_plan(ctx)?;
        let task = self.selected_task(plan)?;
        let task_id = task.task_id.clone();
        let tab = crate::components::detail_pane::DetailTab::Stream;

        let executor_id = task.delegation_id.as_ref().and_then(|did| {
            ctx.lineage
                .executor_id_for_delegation(&spur_acp::domain::delegation::DelegationId(
                    did.clone(),
                ))
                .map(|eid| eid.0.clone())
        });

        match (key.code, key.modifiers) {
            (KeyCode::Char('s'), KeyModifiers::NONE) => match executor_id {
                Some(executor_id) => {
                    self.enter_stream_peek(executor_id, task_id);
                    None
                }
                None => Some(Action::FlashHint {
                    message: format!("Task {task_id} has no active worker yet"),
                }),
            },
            (KeyCode::Char('S'), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                match executor_id {
                    Some(executor_id) => Some(Action::FocusWorkerInDashboard { executor_id, tab }),
                    None => Some(Action::FlashHint {
                        message: format!("Task {task_id} has no active worker yet"),
                    }),
                }
            }
            _ => None,
        }
    }
}

fn is_terminal_phase(phase: LifecycleState) -> bool {
    matches!(
        phase,
        LifecycleState::Succeeded | LifecycleState::Failed | LifecycleState::Cancelled
    )
}

impl View for PlanInspectorView {
    fn handle_key(&mut self, key: KeyEvent, ctx: &super::ViewContext) -> Option<Action> {
        let key = super::normalize_macos_option(key);
        if matches!(self.confirm, Some(PlanInspectorConfirm::RetryTask { .. })) {
            return self.handle_prompt_confirm_key(key);
        }
        if self.confirm.is_some() {
            return match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => self.confirm_action(),
                KeyCode::Esc | KeyCode::Char('q') if key.modifiers.is_empty() => {
                    self.confirm = None;
                    None
                }
                _ => None,
            };
        }

        let plan = self.active_plan(ctx);
        if let Some(plan) = plan {
            self.ensure_selection(plan);

            if key.code == KeyCode::Tab && key.modifiers.is_empty() {
                if self.open_issue_id.is_none() {
                    self.detail_focused = !self.detail_focused;
                }
                return None;
            }

            if self.open_issue_id.is_some() || self.detail_focused {
                match key.code {
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.scroll_task_detail_up(1);
                        return Some(Action::ScrollUp);
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.scroll_task_detail_down(1);
                        return Some(Action::ScrollDown);
                    }
                    KeyCode::Char('g') if key.modifiers.is_empty() => {
                        self.scroll_task_detail_to_top();
                        return Some(Action::ScrollToTop);
                    }
                    KeyCode::Char('G') => {
                        self.scroll_task_detail_to_bottom();
                        return Some(Action::ScrollToBottom);
                    }
                    KeyCode::PageUp => {
                        self.scroll_task_detail_up(10);
                        return Some(Action::ScrollUp);
                    }
                    KeyCode::PageDown => {
                        self.scroll_task_detail_down(10);
                        return Some(Action::ScrollDown);
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.move_lane(plan, -1),
                KeyCode::Char('l') | KeyCode::Right => self.move_lane(plan, 1),
                KeyCode::Char('j') | KeyCode::Down => self.move_task(plan, 1),
                KeyCode::Char('k') | KeyCode::Up => self.move_task(plan, -1),
                KeyCode::Char('g') if key.modifiers.is_empty() => self.jump_lane_start(plan),
                KeyCode::Char('G') => self.jump_lane_end(plan),
                KeyCode::Char('b') if key.modifiers.is_empty() => {
                    return self.jump_to_next_blocker(plan);
                }
                KeyCode::Char('r') if key.modifiers.is_empty() => {
                    return self.start_or_resume_plan(plan);
                }
                KeyCode::Char('R')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    return self.retry_selected_task(plan);
                }
                KeyCode::Char('X')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    return self.stop_selected_task(plan);
                }
                KeyCode::Char(ch @ ('a' | 'd' | 'c'))
                    if key.modifiers.is_empty()
                        && self
                            .selected_task(plan)
                            .is_some_and(|task| is_reviewable_task_status(&task.status)) =>
                {
                    return self.review_selected_task(plan, ctx.lineage, ch);
                }
                KeyCode::Enter if key.modifiers.is_empty() => {
                    if let Some(task) = self.selected_task(plan) {
                        if self.selected_issue_id(task).is_some() {
                            return self.toggle_issue_detail(task);
                        }
                        return Some(Action::FlashHint {
                            message: "No issue linked to selected task".to_string(),
                        });
                    }
                }
                KeyCode::Char('o') if key.modifiers.is_empty() => {
                    if let Some(epic_id) = plan.epic_id.as_ref() {
                        return Some(Action::OpenIssueInBacklog {
                            id: epic_id.clone(),
                        });
                    }
                    return Some(Action::FlashHint {
                        message: "No source work item linked to this implementation plan snapshot"
                            .into(),
                    });
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc => {
                if self.open_issue_id.is_some() {
                    self.close_issue_detail();
                    None
                } else {
                    Some(Action::NavigateBack)
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::ALT) => {
                Some(Action::NavigateBack)
            }
            _ => None,
        }
    }

    fn handle_spur_event(&mut self, event: &SpurEvent, ctx: &super::ViewContext) {
        match &event.body {
            spur_acp::SpurEventBody::IssueDetailFetched {
                requested_id,
                issue,
            } => {
                self.issue_states.insert(
                    requested_id.clone(),
                    TaskIssueState::Loaded(Box::new(Self::detail_event_to_issue(issue))),
                );
            }
            spur_acp::SpurEventBody::IssueCommandError {
                operation,
                error,
                id: Some(id),
            } if operation == "GetIssueDetail" => {
                if let Some(TaskIssueState::Loading) = self.issue_states.get(id) {
                    self.issue_states
                        .insert(id.clone(), TaskIssueState::Error(error.clone()));
                }
            }
            spur_acp::SpurEventBody::IssueCommandError {
                operation,
                error,
                id: None,
            } if operation == "GetIssueDetail" => {
                if let Some(open_issue_id) = self.open_issue_id.as_ref() {
                    let loading_count = self
                        .issue_states
                        .values()
                        .filter(|state| matches!(state, TaskIssueState::Loading))
                        .count();
                    if loading_count == 1
                        && matches!(
                            self.issue_states.get(open_issue_id),
                            Some(TaskIssueState::Loading)
                        )
                    {
                        self.issue_states
                            .insert(open_issue_id.clone(), TaskIssueState::Error(error.clone()));
                    }
                }
            }
            spur_acp::SpurEventBody::PlanCommandError {
                operation,
                plan_id,
                error,
            } => {
                let displayed_plan_id = self.pinned_plan_id.as_deref().or_else(|| {
                    ctx.plan_projection
                        .current_for_session(&self.session_id)
                        .map(|plan| plan.plan_id.as_str())
                });
                if matches!(
                    (displayed_plan_id, plan_id.as_deref()),
                    (Some(displayed), Some(event_plan)) if displayed != event_plan
                ) {
                    return;
                }
                let display_error = error
                    .split_once(": ")
                    .map(|(prefix, rest)| {
                        if prefix.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                            rest
                        } else {
                            error.as_str()
                        }
                    })
                    .unwrap_or(error.as_str());
                let message = match plan_id {
                    Some(plan_id) => {
                        format!("{operation} blocked for {plan_id}: {display_error}")
                    }
                    None => format!("{operation} blocked: {display_error}"),
                };
                self.loop_event_status = Some(PlanInspectorEventStatus::CommandError(message));
            }
            spur_acp::SpurEventBody::LoopArmed {
                loop_id,
                generation,
                next_run,
            } => {
                self.loop_event_status = Some(PlanInspectorEventStatus::Loop(format!(
                    "loop {loop_id} gen {generation}: armed next {next_run}"
                )));
            }
            spur_acp::SpurEventBody::LoopGenerationStarted {
                loop_id,
                generation,
                plan_id,
            } => {
                self.loop_event_status = Some(PlanInspectorEventStatus::Loop(format!(
                    "loop {loop_id} gen {generation}: plan {plan_id}"
                )));
            }
            spur_acp::SpurEventBody::LoopRunRecorded {
                loop_id,
                generation,
                outcome,
                cost_micros,
            } => {
                self.loop_event_status = Some(PlanInspectorEventStatus::Loop(format!(
                    "loop {loop_id} gen {generation}: [{outcome}] {cost_micros} micros"
                )));
            }
            spur_acp::SpurEventBody::LoopPaused { loop_id, by } => {
                self.loop_event_status = Some(PlanInspectorEventStatus::Loop(format!(
                    "loop {loop_id}: {by}"
                )));
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &super::ViewContext) {
        let chunks = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
        self.stacked_mode = area.width < 90;

        let (selected_task_has_issue, selected_task_available) = if let Some(plan) =
            self.active_plan(ctx)
        {
            self.ensure_selection(plan);
            let selected = self.selected_task(plan);
            let selected_task_available = selected.is_some();
            let selected_stage_idx = selected.map(|t| t.stage_idx).unwrap_or(0);
            let live_node =
                selected
                    .and_then(|task| task.issue_id.as_deref())
                    .and_then(|issue_id| {
                        crate::components::plan_stage_board::preferred_live_node(
                            ctx.lineage,
                            issue_id,
                        )
                    });
            let issue_detail = selected
                .as_ref()
                .map(|task| self.issue_detail_for_selected(task))
                .unwrap_or((None, None));

            // ── Header ──────────────────────────────────────────────────────
            let header_rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(chunks[0]);
            let header_cols =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(header_rows[0]);

            let status_color = plan_status_color(ctx.theme, &plan.status);
            let mut header_spans = vec![
                Span::styled(
                    format!(" Plan: {} ", plan.plan_id),
                    Style::default()
                        .fg(token(ctx.theme, "plan_inspector.title.fg"))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {} ", plan.status.to_uppercase()),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(origin) = ctx.plan_projection.loop_origin(&plan.plan_id) {
                header_spans.push(Span::styled(
                    format!(" {} ", loop_origin_badge(origin)),
                    Style::default()
                        .fg(token(ctx.theme, "plan_inspector.title.fg"))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            header_spans.push(Span::raw(format!("  {} ", plan.progress)));
            frame.render_widget(Paragraph::new(Line::from(header_spans)), header_cols[0]);

            let total_tasks = plan.counts.pending
                + plan.counts.ready
                + plan.counts.dispatched
                + plan.counts.awaiting_review
                + plan.counts.approved
                + plan.counts.rejected
                + plan.counts.failed
                + plan.counts.escalated
                + plan.counts.cancelled;
            let gauge_ratio = if total_tasks == 0 {
                0.0
            } else {
                plan.counts.approved as f64 / total_tasks as f64
            };
            frame.render_widget(
                Gauge::default()
                    .ratio(gauge_ratio.clamp(0.0, 1.0))
                    .label(format!(
                        "{} / {} approved",
                        plan.counts.approved, total_tasks
                    ))
                    .gauge_style(
                        Style::default().fg(token(ctx.theme, "plan_inspector.gauge.fill.fg")),
                    )
                    .style(Style::default().fg(token(ctx.theme, "plan_inspector.gauge.track.fg"))),
                header_cols[1],
            );

            let (summary_label, summary_text) = operator_summary(plan, ctx.lineage);
            let summary_width =
                area.width
                    .saturating_sub(summary_label.chars().count() as u16) as usize;
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        summary_label,
                        Style::default().fg(token(ctx.theme, "plan_inspector.label.fg")),
                    ),
                    Span::styled(
                        truncate_display(&summary_text, summary_width),
                        Style::default()
                            .fg(token(ctx.theme, "plan_inspector.header.next_action.fg")),
                    ),
                ])),
                header_rows[1],
            );

            frame.render_widget(
                Paragraph::new(Line::from(header_count_spans(plan, ctx.theme))),
                header_rows[2],
            );

            let epic_text = plan
                .epic_id
                .as_deref()
                .map(|epic_id| format!("work item: {epic_id}"))
                .unwrap_or_else(|| "work item: --".into());
            let owner_text = plan
                .owner_brain_session_id
                .as_deref()
                .map(|owner| format!("owner: {owner}"))
                .unwrap_or_else(|| "owner: --".into());
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " source: ",
                        Style::default().fg(token(ctx.theme, "plan_inspector.label.fg")),
                    ),
                    Span::raw(epic_text),
                    Span::styled(
                        "  ",
                        Style::default().fg(token(ctx.theme, "plan_inspector.label.fg")),
                    ),
                    Span::raw(owner_text),
                ])),
                header_rows[3],
            );

            if !self.stacked_mode {
                let [board_area, detail_area] =
                    Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                        .areas(chunks[1]);
                if let Some(selected_task_id) = self.selected_task_id.as_deref() {
                    crate::components::plan_stage_board::render_stage_board(
                        frame,
                        board_area,
                        plan,
                        selected_task_id,
                        selected_stage_idx,
                        ctx.lineage,
                        ctx.theme,
                    );
                }
                if let Some(task) = selected {
                    let scroll = crate::components::plan_task_detail::render_task_detail(
                        frame,
                        detail_area,
                        plan,
                        task,
                        live_node,
                        issue_detail.0,
                        issue_detail.1,
                        if self.task_detail_to_bottom {
                            usize::MAX
                        } else {
                            self.task_detail_scroll
                        },
                        ctx.theme,
                    );
                    self.task_detail_scroll = scroll.offset;
                    self.task_detail_max_scroll = scroll.max_offset;
                    self.task_detail_to_bottom = false;
                } else {
                    frame.render_widget(
                        Paragraph::new("No task selected")
                            .block(Block::default().borders(Borders::ALL).title("Task detail")),
                        detail_area,
                    );
                }
            } else {
                let [board_area, selected_area, detail_area] = Layout::vertical([
                    Constraint::Percentage(55),
                    Constraint::Length(1),
                    Constraint::Percentage(45),
                ])
                .areas(chunks[1]);
                if let Some(selected_task_id) = self.selected_task_id.as_deref() {
                    crate::components::plan_stage_board::render_stacked_stage_groups(
                        frame,
                        board_area,
                        plan,
                        selected_task_id,
                        ctx.lineage,
                        ctx.theme,
                    );
                }
                frame.render_widget(
                    Paragraph::new(format!(
                        "Selected: {}",
                        selected
                            .map(|task| task.task_name.as_str())
                            .unwrap_or("(none)")
                    )),
                    selected_area,
                );
                if let Some(task) = selected {
                    let scroll = crate::components::plan_task_detail::render_task_detail(
                        frame,
                        detail_area,
                        plan,
                        task,
                        live_node,
                        issue_detail.0,
                        issue_detail.1,
                        if self.task_detail_to_bottom {
                            usize::MAX
                        } else {
                            self.task_detail_scroll
                        },
                        ctx.theme,
                    );
                    self.task_detail_scroll = scroll.offset;
                    self.task_detail_max_scroll = scroll.max_offset;
                    self.task_detail_to_bottom = false;
                }
            }

            (
                selected.and_then(|task| task.issue_id.as_deref()).is_some(),
                selected_task_available,
            )
        } else {
            let message = self
                .pinned_plan_id
                .as_ref()
                .map(|plan_id| {
                    format!("No tracked implementation plan snapshot for {plan_id} yet.")
                })
                .unwrap_or_else(|| "No tracked plan for this session yet.".into());
            frame.render_widget(
                Paragraph::new(message).block(Block::default().borders(Borders::ALL)),
                chunks[1],
            );
            (false, false)
        };
        let selected_task_retryable = self
            .active_plan(ctx)
            .and_then(|plan| self.selected_task(plan))
            .map(is_retryable_task)
            .unwrap_or(false);
        let selected_task_stoppable = self
            .active_plan(ctx)
            .and_then(|plan| self.selected_task(plan))
            .map(is_stoppable_task)
            .unwrap_or(false);
        let selected_task_reviewable = self
            .active_plan(ctx)
            .and_then(|plan| self.selected_task(plan))
            .map(|task| is_reviewable_task_status(&task.status))
            .unwrap_or(false);

        let enter_hint = if self.open_issue_id.is_some() {
            "Enter: close issue detail"
        } else if selected_task_has_issue {
            "Enter: issue detail"
        } else {
            "Enter: no linked issue"
        };
        let scroll_hint = if self.open_issue_id.is_some() {
            "  j/k: line scroll  g/G: top/btm  PgUp/PgDn: page"
        } else {
            ""
        };
        let peek_hint = if self.open_issue_id.is_none() && selected_task_available {
            "  s: stream peek  S: jump to worker"
        } else {
            ""
        };
        let retry_hint = if self.open_issue_id.is_none() && selected_task_retryable {
            "  R: retry"
        } else {
            ""
        };
        let stop_hint = if self.open_issue_id.is_none() && selected_task_stoppable {
            "  X: stop"
        } else {
            ""
        };
        let review_hint = if self.open_issue_id.is_none() && selected_task_reviewable {
            "  a: approve  d: reject  c: changes"
        } else {
            ""
        };
        let footer = footer_hint(
            area.width,
            enter_hint,
            peek_hint,
            retry_hint,
            stop_hint,
            review_hint,
            scroll_hint,
            self.open_issue_id.is_none() && self.active_plan(ctx).is_some(),
            self.loop_event_status
                .as_ref()
                .map(PlanInspectorEventStatus::message),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                footer,
                Style::default().fg(token(ctx.theme, "plan_inspector.footer_hint.fg")),
            )])),
            chunks[2],
        );
        self.render_confirm(frame, area, ctx.theme);
    }

    fn tick(&mut self) {}
}

impl PlanInspectorView {
    fn render_confirm(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(confirm) = self.confirm.as_ref() else {
            return;
        };
        let popup_height = match confirm {
            PlanInspectorConfirm::Review {
                feedback: Some(feedback),
                ..
            } if !feedback.trim().is_empty() => 10,
            PlanInspectorConfirm::RetryTask { .. } => 10,
            PlanInspectorConfirm::StopTask { .. } => 11,
            _ => 9,
        };
        let popup = centered_rect(area, 72, popup_height);
        frame.render_widget(Clear, popup);

        let (title, verb, body) = match confirm {
            PlanInspectorConfirm::Start { plan_id } => (
                " Start / Resume Plan ",
                "Start",
                vec![
                    Line::from(format!("  Plan: {plan_id}")),
                    Line::from(""),
                    Line::from("  This starts/resumes execution in the current brain session."),
                    Line::from("  A brain session can actively execute only one plan."),
                    Line::from("  Workers may be dispatched after this starts."),
                ],
            ),
            PlanInspectorConfirm::RetryTask {
                task_id,
                task_name,
                issue_id,
                status,
                attempt,
                max_attempts,
                append_prompt,
                cursor,
                ..
            } => (
                " Retry Task ",
                "Retry",
                vec![
                    Line::from(format!("  Task: {task_id} - {task_name}")),
                    Line::from(format!("  Issue: {issue_id}")),
                    Line::from(format!("  Status: {status}")),
                    Line::from(format!("  Attempt: {attempt}/{max_attempts}")),
                    prompt_input_line(append_prompt, *cursor, theme),
                    Line::from("  This requeues the selected task for another worker attempt."),
                ],
            ),
            PlanInspectorConfirm::StopTask {
                plan_id,
                task_id,
                task_name,
                delegation_id,
                status,
                retry_guidance,
                ..
            } => (
                " Stop Task ",
                "Stop",
                vec![
                    Line::from(format!("  Plan: {plan_id}")),
                    Line::from(format!("  Task: {task_id} - {task_name}")),
                    Line::from(format!("  Delegation: {delegation_id}")),
                    Line::from(format!("  Status: {status}")),
                    Line::from(""),
                    Line::from("  This cancels the active worker."),
                    Line::from(format!("  {retry_guidance}")),
                ],
            ),
            PlanInspectorConfirm::Review {
                plan_id,
                task_id,
                task_name,
                status,
                attempt_n,
                decision,
                feedback,
                ..
            } => {
                let mut lines = vec![
                    Line::from(format!("  Plan: {plan_id}")),
                    Line::from(format!("  Task: {task_id} - {task_name}")),
                    Line::from(format!("  Status: {status}")),
                    Line::from(format!("  Attempt: {attempt_n}")),
                    Line::from(format!("  Decision: {}", review_decision_verb(decision))),
                ];
                if let Some(feedback) = feedback.as_ref().filter(|text| !text.trim().is_empty()) {
                    let max_feedback = popup.width.saturating_sub(14) as usize;
                    lines.push(Line::from(format!(
                        "  Feedback: {}",
                        truncate_display(feedback, max_feedback)
                    )));
                }
                (" Review Task ", review_decision_verb(decision), lines)
            }
        };

        let mut lines = body;
        lines.push(Line::from(""));
        lines.push(action_line(
            "[Enter]",
            verb,
            "[Esc]",
            "Cancel",
            popup.width,
            theme,
        ));

        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(token(theme, "plan_browser.confirm.title.fg"))
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Left)
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(token(theme, "plan_browser.confirm.border.fg")));

        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}

fn plan_status_color(theme: &Theme, status: &str) -> Color {
    let key = match status {
        "running" | "active" => "plan_inspector.status.running.fg",
        "approved" | "completed" | "success" => "plan_inspector.status.success.fg",
        "failed" | "rejected" | "error" => "plan_inspector.status.failure.fg",
        "cancelled" => "plan_inspector.status.cancelled.fg",
        _ => "plan_inspector.status.unknown.fg",
    };
    token(theme, key)
}

fn header_count_spans(plan: &TrackedPlan, theme: &Theme) -> Vec<Span<'static>> {
    let count_status = |status: &str| {
        plan.tasks
            .iter()
            .filter(|task| task.status == status)
            .count() as u32
    };
    let setup_conflicts = count_status("blocked_on_setup_conflict");
    let mut spans = Vec::new();
    let mut push_count = |count: u32, label: &'static str, token_name: &'static str| {
        if count == 0 {
            return;
        }
        if !spans.is_empty() {
            spans.push(Span::raw(" · "));
        }
        spans.push(Span::styled(
            format!("{count} {label}"),
            Style::default().fg(token(theme, token_name)),
        ));
    };

    push_count(
        plan.counts.pending.saturating_sub(setup_conflicts),
        "pending",
        "plan_inspector.header.count.pending.fg",
    );
    push_count(
        plan.counts.ready,
        "ready",
        "plan_inspector.header.count.ready.fg",
    );
    push_count(
        plan.counts.dispatched,
        "dispatched",
        "plan_inspector.header.count.dispatched.fg",
    );
    push_count(
        plan.counts.awaiting_review,
        "awaiting_review",
        "plan_inspector.header.count.awaiting_review.fg",
    );
    push_count(
        plan.counts.approved,
        "approved",
        "plan_inspector.header.count.approved.fg",
    );
    push_count(
        plan.counts.rejected,
        "rejected",
        "plan_inspector.header.count.rejected.fg",
    );
    push_count(
        plan.counts.failed,
        "failed",
        "plan_inspector.header.count.failed.fg",
    );
    push_count(
        plan.counts.escalated,
        "escalated",
        "plan_inspector.header.count.rejected.fg",
    );
    push_count(
        setup_conflicts,
        "setup conflict",
        "plan_inspector.header.count.blocked.fg",
    );
    push_count(
        count_status("cancelled"),
        "cancelled",
        "plan_inspector.header.count.cancelled.fg",
    );
    push_count(
        count_status("superseded"),
        "superseded",
        "plan_inspector.header.count.cancelled.fg",
    );
    let dependency_blocked = plan
        .tasks
        .iter()
        .filter(|task| !task.blocked_by.is_empty())
        .count();
    if dependency_blocked > 0 {
        spans.push(Span::styled(
            format!(" | deps blocked: {dependency_blocked}"),
            Style::default().fg(token(theme, "plan_inspector.header.count.blocked.fg")),
        ));
    }

    spans
}

fn operator_summary(
    plan: &TrackedPlan,
    lineage: &spur_core::ExecutorLineage,
) -> (&'static str, String) {
    let mut fragments = Vec::new();
    let task_by_id: HashMap<&str, &spur_core::TrackedTask> = plan
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect();
    for task in &plan.tasks {
        if task.blocked_by.is_empty() {
            continue;
        }
        for blocker_id in &task.blocked_by {
            let Some(blocker) = task_by_id.get(blocker_id.as_str()) else {
                continue;
            };
            if matches!(blocker.status.as_str(), "rejected" | "failed" | "error") {
                fragments.push(format!(
                    "{} {} {} ago",
                    blocker.task_id,
                    operator_status_label(&blocker.status),
                    format_age(
                        plan.updated_at
                            .elapsed()
                            .ok()
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    )
                ));
            }
        }
    }

    for task in &plan.tasks {
        if !matches!(task.status.as_str(), "dispatched" | "running" | "active") {
            continue;
        }
        let elapsed_secs = task
            .issue_id
            .as_deref()
            .and_then(|issue_id| {
                crate::components::plan_stage_board::preferred_live_node(lineage, issue_id)
            })
            .map(|node| node.elapsed_secs())
            .unwrap_or_else(|| {
                plan.updated_at
                    .elapsed()
                    .ok()
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });
        if elapsed_secs >= 10 * 60 {
            fragments.push(format!(
                "{} running {}",
                task.task_id,
                format_age(elapsed_secs)
            ));
        }
    }

    for task in &plan.tasks {
        if task.attempt > 1 {
            fragments.push(format!(
                "{} retry {}/{}",
                task.task_id, task.attempt, task.max_attempts
            ));
        }
    }

    if fragments.is_empty() {
        (" next: ", plan.next_action.clone())
    } else {
        (" risk: ", fragments.join(" · "))
    }
}

fn operator_status_label(status: &str) -> &str {
    match status {
        "failed" | "error" => "error",
        other => other,
    }
}

fn is_retryable_task_status(status: &str) -> bool {
    matches!(
        status,
        "failed" | "rejected" | "error" | "cancelled" | "escalated_to_brain"
    )
}

fn is_stoppable_task_status(status: &str) -> bool {
    matches!(status, "dispatched" | "running" | "active")
}

fn is_reviewable_task_status(status: &str) -> bool {
    status == "awaiting_review"
}

fn is_retryable_task(task: &spur_core::TrackedTask) -> bool {
    task.issue_id.is_some()
        && is_retryable_task_status(&task.status)
        && task.attempt < task.max_attempts
}

fn retry_guidance_after_stop(task: &spur_core::TrackedTask) -> String {
    if task.issue_id.is_none() {
        "Retry unavailable: no linked issue.".into()
    } else if task.attempt >= task.max_attempts {
        format!(
            "Retry unavailable: attempt limit reached ({}/{}).",
            task.attempt, task.max_attempts
        )
    } else {
        "Press R afterward to retry.".into()
    }
}

fn is_stoppable_task(task: &spur_core::TrackedTask) -> bool {
    task.delegation_id.is_some() && is_stoppable_task_status(&task.status)
}

fn plan_review_decision_for_key(
    key: char,
    feedback: Option<String>,
) -> Option<spur_core::ReviewDecision> {
    match key {
        'a' => crate::components::review_card::decision_for_key('A', None),
        'd' => crate::components::review_card::decision_for_key(
            'D',
            Some(feedback_or_default(
                feedback,
                "Rejected from Plan Inspector",
            )),
        ),
        'c' => Some(spur_core::ReviewDecision::Modify {
            note: feedback_or_default(feedback, "Changes requested from Plan Inspector"),
        }),
        _ => None,
    }
}

fn feedback_or_default(feedback: Option<String>, default: &'static str) -> String {
    feedback
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| default.into())
}

fn review_decision_verb(decision: &spur_core::ReviewDecision) -> &'static str {
    match decision {
        spur_core::ReviewDecision::Approve => "Approve",
        spur_core::ReviewDecision::Reject { .. } => "Reject",
        spur_core::ReviewDecision::Modify { .. } => "Request changes",
        spur_core::ReviewDecision::Retry { .. } => "Retry",
    }
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 60 * 60 {
        format!("{}m", secs / 60)
    } else if secs < 48 * 60 * 60 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn truncate_display(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].trim_end().to_string();
    out.push('…');
    out
}

fn prompt_input_line(text: &str, cursor: usize, theme: &Theme) -> Line<'static> {
    let cursor = clamp_char_boundary(text, cursor);
    Line::from(vec![
        Span::raw("  Append: "),
        Span::raw(text[..cursor].to_string()),
        Span::styled(
            "|",
            Style::default()
                .fg(token(theme, "plan_browser.confirm.primary_key.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(text[cursor..].to_string()),
    ])
}

fn confirm_prompt_mut(
    confirm: &mut Option<PlanInspectorConfirm>,
) -> Option<(&mut String, &mut usize)> {
    match confirm.as_mut()? {
        PlanInspectorConfirm::RetryTask {
            append_prompt,
            cursor,
            ..
        } => Some((append_prompt, cursor)),
        _ => None,
    }
}

fn non_empty_prompt(prompt: String) -> Option<String> {
    if prompt.trim().is_empty() {
        None
    } else {
        Some(prompt)
    }
}

fn insert_at_cursor(text: &mut String, cursor: &mut usize, ch: char) {
    let cursor_pos = clamp_char_boundary(text, *cursor);
    text.insert(cursor_pos, ch);
    *cursor = cursor_pos + ch.len_utf8();
}

fn delete_before_cursor(text: &mut String, cursor: &mut usize) {
    let cursor_pos = clamp_char_boundary(text, *cursor);
    if cursor_pos == 0 {
        *cursor = 0;
        return;
    }
    let previous = previous_char_boundary(text, cursor_pos);
    text.drain(previous..cursor_pos);
    *cursor = previous;
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    if cursor == 0 {
        return 0;
    }
    let mut previous = cursor - 1;
    while previous > 0 && !text.is_char_boundary(previous) {
        previous -= 1;
    }
    previous
}

fn next_char_boundary(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    if cursor >= text.len() {
        return text.len();
    }
    let mut next = cursor + 1;
    while next < text.len() && !text.is_char_boundary(next) {
        next += 1;
    }
    next
}

fn clamp_char_boundary(text: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

#[expect(
    clippy::too_many_arguments,
    reason = "footer hint inputs are independent optional command labels"
)]
fn footer_hint(
    width: u16,
    enter_hint: &str,
    peek_hint: &str,
    retry_hint: &str,
    stop_hint: &str,
    review_hint: &str,
    scroll_hint: &str,
    show_start: bool,
    loop_event_status: Option<&str>,
) -> String {
    let tiers = footer_hint_tiers(
        enter_hint,
        peek_hint,
        retry_hint,
        stop_hint,
        review_hint,
        scroll_hint,
        show_start,
    );
    if let Some(status) = loop_event_status.filter(|status| !status.is_empty()) {
        return footer_hint_with_status(width, status, &tiers);
    }

    footer_hint_without_status(width, &tiers)
}

fn footer_hint_tiers(
    enter_hint: &str,
    peek_hint: &str,
    retry_hint: &str,
    stop_hint: &str,
    review_hint: &str,
    scroll_hint: &str,
    show_start: bool,
) -> [String; 3] {
    let start_hint = if show_start { "  r: start/resume" } else { "" };
    let full = format!(
        " h/l: lane  j/k: task  b: blocker  {}  o: work item{}{}{}{}{}  g/G: ends  Alt+P/Esc: close {}",
        enter_hint, start_hint, retry_hint, stop_hint, review_hint, peek_hint, scroll_hint
    );

    let compact_peek = if peek_hint.is_empty() {
        ""
    } else {
        "  s: peek  S: worker"
    };
    let compact_start = if show_start { "  r: start" } else { "" };
    let compact_retry = if retry_hint.is_empty() {
        ""
    } else {
        "  R: retry"
    };
    let compact_stop = if stop_hint.is_empty() {
        ""
    } else {
        "  X: stop"
    };
    let compact_review = if review_hint.is_empty() {
        ""
    } else {
        "  a/d/c: review"
    };
    let compact = format!(
        " h/l lane  j/k task  b blocker  {}  o item{}{}{}{}{}  Esc close",
        enter_hint, compact_start, compact_retry, compact_stop, compact_review, compact_peek
    );
    let minimal = format!(
        " j/k task  b blocker{}{}{}{}  Esc close",
        compact_start, compact_retry, compact_stop, compact_review
    );

    [full, compact, minimal]
}

fn footer_hint_without_status(width: u16, tiers: &[String; 3]) -> String {
    let width = width as usize;
    for tier in &tiers[..2] {
        if tier.chars().count() <= width {
            return tier.clone();
        }
    }

    truncate_display(&tiers[2], width)
}

fn footer_hint_with_status(width: u16, status: &str, tiers: &[String; 3]) -> String {
    let width = width as usize;
    let separator = "  ";
    let separator_len = separator.chars().count();
    let status_len = status.chars().count();

    for tier in tiers {
        if status_len + separator_len + tier.chars().count() <= width {
            return format!("{status}{separator}{tier}");
        }
    }

    let minimal = &tiers[2];
    let minimal_len = minimal.chars().count();
    if minimal_len + separator_len >= width {
        return truncate_display(minimal, width);
    }

    let status_budget = width - minimal_len - separator_len;
    let status = truncate_footer_status(status, status_budget);
    format!("{status}{separator}{minimal}")
}

fn truncate_footer_status(status: &str, max_chars: usize) -> String {
    let char_count = status.chars().count();
    if char_count <= max_chars {
        return status.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut out = status
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn centered_rect(outer: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(outer.width);
    let height = height.min(outer.height);
    Rect {
        x: outer.x + outer.width.saturating_sub(width) / 2,
        y: outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn action_line(
    left_key: &'static str,
    left_label: &'static str,
    right_key: &'static str,
    right_label: &'static str,
    popup_width: u16,
    theme: &Theme,
) -> Line<'static> {
    let left_width = 1 + left_key.len() + 1 + left_label.len();
    let right_width = right_key.len() + 1 + right_label.len();
    let content_width = popup_width.saturating_sub(2) as usize;
    let gap = content_width
        .saturating_sub(left_width + right_width)
        .max(1);

    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            left_key,
            Style::default()
                .fg(token(theme, "plan_browser.confirm.primary_key.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {left_label}{}", " ".repeat(gap))),
        Span::styled(
            right_key,
            Style::default()
                .fg(token(theme, "plan_browser.confirm.cancel_key.fg"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {right_label}")),
    ])
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use spur_acp::{
        LifecycleState, PlanSnapshot, PlanSnapshotCounts, PlanSnapshotTask, ReviewKind,
        ReviewPayload, SessionId, SpurEvent, SpurEventBody,
    };
    use spur_core::{ExecutorLineage, PlanProjectionStore, SessionSynopsisProjection};

    use crate::action::Action;
    use crate::app::BrainStatus;
    use crate::views::{View, ViewContext};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn key_shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    fn key_code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ctx<'a>(
        lineage: &'a ExecutorLineage,
        projection: &'a PlanProjectionStore,
        synopsis: &'a SessionSynopsisProjection,
    ) -> ViewContext<'a> {
        ViewContext {
            lineage,
            plan_projection: projection,
            synopsis,
            brain_status: &BrainStatus::Idle,
            flag_summary: None,
            tombstone: None,
            transient_hint_override: None,
            notebook_ready: false,
            theme: crate::theme::fallback_theme(),
        }
    }

    fn view_context_for_tests<'a>(
        lineage: &'a ExecutorLineage,
        projection: &'a PlanProjectionStore,
    ) -> ViewContext<'a> {
        let synopsis = Box::leak(Box::new(SessionSynopsisProjection::new()));
        ctx(lineage, projection, synopsis)
    }

    fn buffer_row(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        row
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            if y > 0 {
                text.push('\n');
            }
            text.push_str(&buffer_row(buf, y));
        }
        text
    }

    fn find_text_cell(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
        for y in 0..buf.area.height {
            let row = buffer_row(buf, y);
            if let Some(byte_x) = row.find(needle) {
                let x = row[..byte_x].chars().count() as u16;
                return Some((x, y));
            }
        }
        None
    }

    fn projection_with_epic(session_id: &SessionId) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action: "dispatch first stage".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    pending: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-epic.1".into()),
                    issue_title: None,
                    status: "pending".into(),
                    attempt: 0,
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
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_terminal_plan(session_id: &SessionId) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "approved".into(),
                progress: "1/1 done".into(),
                next_action: "terminal".into(),
                ready_to_merge: true,
                counts: PlanSnapshotCounts {
                    approved: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-epic.1".into()),
                    issue_title: None,
                    status: "approved".into(),
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
                    next_action: "done".into(),
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_single_task_status(
        session_id: &SessionId,
        status: &str,
        issue_id: Option<&str>,
        attempt: u32,
        max_attempts: u32,
    ) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action: "inspect failed task".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    failed: u32::from(status == "failed"),
                    rejected: u32::from(status == "rejected"),
                    pending: u32::from(status == "pending"),
                    dispatched: u32::from(matches!(status, "running" | "dispatched")),
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: issue_id.map(str::to_string),
                    issue_title: None,
                    status: status.into(),
                    attempt,
                    max_attempts,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                    unblocks: Vec::new(),
                    summary: None,
                    feedback: None,
                    error: (status == "error").then(|| "worker error".into()),
                    worker_branch: None,
                    delegation_id: None,
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: Vec::new(),
                    next_action: "operator retry".into(),
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_awaiting_review_task(
        session_id: &SessionId,
        feedback: Option<&str>,
        delegation_id: Option<&str>,
    ) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action: "review worker output".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    awaiting_review: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-epic.1".into()),
                    issue_title: None,
                    status: "awaiting_review".into(),
                    attempt: 2,
                    max_attempts: 3,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                    unblocks: Vec::new(),
                    summary: Some("Worker says Stage A is ready".into()),
                    feedback: feedback.map(str::to_string),
                    error: None,
                    worker_branch: Some("worker/stage-a".into()),
                    delegation_id: delegation_id.map(str::to_string),
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: Vec::new(),
                    next_action: "operator review".into(),
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_epic_and_worker(session_id: &SessionId) -> PlanProjectionStore {
        projection_with_epic_and_worker_for_delegation(session_id, "deleg-12")
    }

    fn projection_with_epic_and_worker_for_delegation(
        session_id: &SessionId,
        delegation_id: &str,
    ) -> PlanProjectionStore {
        projection_with_stoppable_task(session_id, delegation_id, 1, 3)
    }

    fn projection_with_stoppable_task(
        session_id: &SessionId,
        delegation_id: &str,
        attempt: u32,
        max_attempts: u32,
    ) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action: "watch worker".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    dispatched: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-epic.1".into()),
                    issue_title: None,
                    status: "dispatched".into(),
                    attempt,
                    max_attempts,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                    unblocks: Vec::new(),
                    summary: None,
                    feedback: None,
                    error: None,
                    worker_branch: None,
                    delegation_id: Some(delegation_id.into()),
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: Vec::new(),
                    next_action: "watch stream".into(),
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_two_running_tasks(session_id: &SessionId) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/2 done".into(),
                next_action: "watch workers".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    dispatched: 2,
                    ..Default::default()
                },
                tasks: vec![
                    PlanSnapshotTask {
                        task_id: "t-12".into(),
                        task_name: "Stage A".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-epic.1".into()),
                        issue_title: None,
                        status: "dispatched".into(),
                        attempt: 1,
                        max_attempts: 3,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                        unblocks: Vec::new(),
                        summary: None,
                        feedback: None,
                        error: None,
                        worker_branch: None,
                        delegation_id: Some("deleg-12".into()),
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: Vec::new(),
                        next_action: "watch stream".into(),
                    },
                    PlanSnapshotTask {
                        task_id: "t-13".into(),
                        task_name: "Stage B".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-epic.2".into()),
                        issue_title: None,
                        status: "dispatched".into(),
                        attempt: 1,
                        max_attempts: 3,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                        unblocks: Vec::new(),
                        summary: None,
                        feedback: None,
                        error: None,
                        worker_branch: None,
                        delegation_id: Some("deleg-13".into()),
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: Vec::new(),
                        next_action: "watch stream".into(),
                    },
                ],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn projection_with_header_counts(session_id: &SessionId) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/6 done".into(),
                next_action: "watch active work".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    dispatched: 3,
                    awaiting_review: 2,
                    failed: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "t-12".into(),
                    task_name: "Stage A".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-epic.1".into()),
                    issue_title: None,
                    status: "dispatched".into(),
                    attempt: 1,
                    max_attempts: 3,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                    unblocks: Vec::new(),
                    summary: None,
                    feedback: None,
                    error: None,
                    worker_branch: None,
                    delegation_id: Some("deleg-12".into()),
                    diff_summary: None,
                    mutation_id: None,
                    superseded_by: Vec::new(),
                    next_action: "watch stream".into(),
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    fn lineage_with_worker_for_task(
        session_id: &SessionId,
        task_id: &str,
        worker_session: &str,
    ) -> ExecutorLineage {
        lineage_with_worker_for_task_and_delegation(session_id, task_id, worker_session, "deleg-12")
    }

    fn lineage_with_worker_for_task_and_delegation(
        session_id: &SessionId,
        task_id: &str,
        worker_session: &str,
        delegation_id: &str,
    ) -> ExecutorLineage {
        let mut lineage = ExecutorLineage::new();
        lineage.apply(&SpurEvent::now(SpurEventBody::WorkerSpawned {
            agent: "codex".into(),
            session: SessionId(worker_session.into()),
            worktree: std::path::PathBuf::from("/tmp"),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
            from: session_id.clone(),
            to_agent: "codex".into(),
            task: task_id.into(),
            request_id: delegation_id.into(),
            delegation_plan: None,
            issue_id: Some("bd-epic.1".into()),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
            from: session_id.clone(),
            request_id: delegation_id.into(),
            executor_id: worker_session.into(),
        }));
        lineage
    }

    fn lineage_with_pending_review_for_delegation(
        session_id: &SessionId,
        task_id: &str,
        worker_session: &str,
        delegation_id: &str,
        attempt_n: u32,
    ) -> ExecutorLineage {
        let mut lineage = lineage_with_worker_for_task_and_delegation(
            session_id,
            task_id,
            worker_session,
            delegation_id,
        );
        lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorReviewRequested {
            id: worker_session.into(),
            attempt_n,
            kind: ReviewKind::Completion,
            payload: ReviewPayload {
                summary: "worker ready for review".into(),
                diff_summary: None,
                pr_url: None,
                error: None,
                delegation_plan: None,
                chosen_matches_dispatched: None,
                peer_influence: None,
            },
        }));
        lineage
    }

    fn lineage_with_two_workers(session_id: &SessionId) -> ExecutorLineage {
        let mut lineage = ExecutorLineage::new();
        lineage.apply(&SpurEvent::now(SpurEventBody::WorkerSpawned {
            agent: "codex".into(),
            session: SessionId("worker-session-1".into()),
            worktree: std::path::PathBuf::from("/tmp/worker-1"),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::WorkerSpawned {
            agent: "codex".into(),
            session: SessionId("worker-session-2".into()),
            worktree: std::path::PathBuf::from("/tmp/worker-2"),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
            from: session_id.clone(),
            to_agent: "codex".into(),
            task: "t-12".into(),
            request_id: "deleg-12".into(),
            delegation_plan: None,
            issue_id: Some("bd-epic.1".into()),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationRequested {
            from: session_id.clone(),
            to_agent: "codex".into(),
            task: "t-13".into(),
            request_id: "deleg-13".into(),
            delegation_plan: None,
            issue_id: Some("bd-epic.2".into()),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
            from: session_id.clone(),
            request_id: "deleg-12".into(),
            executor_id: "worker-session-1".into(),
        }));
        lineage.apply(&SpurEvent::now(SpurEventBody::DelegationDispatched {
            from: session_id.clone(),
            request_id: "deleg-13".into(),
            executor_id: "worker-session-2".into(),
        }));
        lineage
    }

    fn projection_with_blocker_cycle(session_id: &SessionId) -> PlanProjectionStore {
        let mut projection = PlanProjectionStore::new();
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "plan-1".into(),
                epic_id: Some("bd-epic".into()),
                status: "running".into(),
                progress: "0/3 done".into(),
                next_action: "resolve blockers".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    pending: 1,
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
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                        unblocks: vec!["ui".into()],
                        summary: None,
                        feedback: None,
                        error: None,
                        worker_branch: None,
                        delegation_id: None,
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: Vec::new(),
                        next_action: "fix lint".into(),
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
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                        unblocks: vec!["ui".into()],
                        summary: None,
                        feedback: None,
                        error: Some("build failed".into()),
                        worker_branch: None,
                        delegation_id: None,
                        diff_summary: None,
                        mutation_id: None,
                        superseded_by: Vec::new(),
                        next_action: "retry".into(),
                    },
                    PlanSnapshotTask {
                        task_id: "ui".into(),
                        task_name: "ui".into(),
                        agent: "codex".into(),
                        issue_id: Some("bd-ui".into()),
                        issue_title: None,
                        status: "pending".into(),
                        attempt: 0,
                        max_attempts: 3,
                        depends_on: vec!["lint".into(), "api".into()],
                        blocked_by: vec!["lint".into(), "api".into()],
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
                    },
                ],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        projection
    }

    #[test]
    fn new_view_starts_in_browse_mode() {
        let v = PlanInspectorView::new(SessionId("s".into()));
        assert!(matches!(v.mode(), PlanInspectorMode::Browse));
    }

    #[test]
    fn enter_stream_peek_sets_mode_and_initial_state() {
        let mut v = PlanInspectorView::new(SessionId("s".into()));
        v.enter_stream_peek("worker-session-1".into(), "t-12".into());
        match v.mode() {
            PlanInspectorMode::StreamPeek {
                executor_id,
                task_id,
                state,
            } => {
                assert_eq!(executor_id, "worker-session-1");
                assert_eq!(task_id, "t-12");
                assert!(state.is_following);
                assert_eq!(state.scroll_offset, 0);
            }
            _ => panic!("expected StreamPeek"),
        }
    }

    #[test]
    fn leave_stream_peek_returns_to_browse() {
        let mut v = PlanInspectorView::new(SessionId("s".into()));
        v.enter_stream_peek("w".into(), "t".into());
        v.leave_stream_peek();
        assert!(matches!(v.mode(), PlanInspectorMode::Browse));
    }

    #[test]
    fn render_with_worker_streams_does_not_panic_when_no_streams() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic(&session_id);
        let lineage = ExecutorLineage::new();
        let synopsis = SessionSynopsisProjection::new();
        let ctx = ctx(&lineage, &projection, &synopsis);
        let mut ws = crate::worker_streams::WorkerStreams::new();
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn peek_overlay_renders_in_60_col_terminal_without_panic() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let dump = format!("{:?}", buf);
        assert!(
            dump.contains("stream:"),
            "expected 'stream:' title in buffer"
        );
        assert!(dump.contains("t-12"), "expected task id in buffer");
    }

    #[test]
    fn peek_overlay_draws_rounded_popup_side_borders() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let popup = Rect::new(8, 5, 64, 14);
        assert_eq!(buf[(popup.x, popup.y)].symbol(), "╭");
        assert_eq!(buf[(popup.x + popup.width - 1, popup.y)].symbol(), "╮");
        assert_eq!(buf[(popup.x, popup.y + 1)].symbol(), "│");
        assert_eq!(buf[(popup.x + popup.width - 1, popup.y + 1)].symbol(), "│");
    }

    #[test]
    fn footer_advertises_peek_keys_in_browse_mode() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let backend = ratatui::backend::TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            dump.contains("s: stream peek"),
            "expected footer to advertise stream peek:\n{dump}"
        );
        assert!(
            dump.contains("S: jump to worker"),
            "expected footer to advertise dashboard jump:\n{dump}"
        );
        assert!(
            dump.contains("r: start/resume"),
            "expected footer to advertise start/resume:\n{dump}"
        );
    }

    #[test]
    fn header_renders_nonzero_status_counts_with_tokens() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_header_counts(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let backend = ratatui::backend::TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let text = buffer_text(buf);
        assert!(
            text.contains("3 dispatched"),
            "expected header count strip to show dispatched count:\n{text}"
        );
        assert!(
            text.contains("2 awaiting_review"),
            "expected header count strip to show awaiting_review count:\n{text}"
        );
        assert!(
            text.contains("1 failed"),
            "expected header count strip to show failed count:\n{text}"
        );
        assert!(
            !text.contains("0 pending"),
            "expected header count strip to omit zero counts:\n{text}"
        );

        let (x, y) = find_text_cell(buf, "3 dispatched")
            .expect("expected dispatched count position in rendered buffer");
        assert_eq!(
            buf[(x, y)].style().fg,
            Some(token(
                ctx.theme,
                "plan_inspector.header.count.dispatched.fg"
            ))
        );
        let (x, y) =
            find_text_cell(buf, "1 failed").expect("expected failed count position in buffer");
        assert_eq!(
            buf[(x, y)].style().fg,
            Some(token(ctx.theme, "plan_inspector.header.count.failed.fg"))
        );
    }

    #[test]
    fn header_labels_risk_summary_as_risk_not_next() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_blocker_cycle(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("ui".into()));

        let backend = ratatui::backend::TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("risk: lint rejected"),
            "expected header to render the risk summary:\n{text}"
        );
        assert!(
            !text.contains("next: risk:"),
            "expected risk summary not to be mislabeled as next action:\n{text}"
        );
    }

    #[test]
    fn footer_uses_compact_start_hint_on_narrow_terminals() {
        let hint = footer_hint(
            50,
            "Enter: issue detail",
            "  s: stream peek  S: jump to worker",
            "",
            "",
            "",
            "",
            true,
            None,
        );

        assert!(
            hint.contains("r: start"),
            "expected compact footer to advertise start:\n{hint}"
        );
        assert!(
            hint.chars().count() <= 50,
            "expected compact footer to fit narrow width:\n{hint}"
        );
    }

    #[test]
    fn footer_preserves_close_hint_when_loop_status_active() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));
        <PlanInspectorView as View>::handle_spur_event(
            &mut view,
            &SpurEvent::now(SpurEventBody::LoopGenerationStarted {
                loop_id: "loop-with-a-long-human-readable-id".into(),
                generation: 42,
                plan_id: "plan-with-a-long-human-readable-id".into(),
            }),
            &ctx,
        );

        let backend = ratatui::backend::TestBackend::new(72, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let footer = buffer_row(terminal.backend().buffer(), 23);
        assert!(
            footer.contains("loop loop-with-a-lo"),
            "expected footer to include loop status:\n{footer}"
        );
        assert!(
            footer.contains("Esc close"),
            "expected loop-prefixed footer to preserve close hint:\n{footer}"
        );
    }

    #[test]
    fn footer_advertises_retry_only_for_retryable_task() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "failed", Some("bd-epic.1"), 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let backend = ratatui::backend::TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            dump.contains("R: retry"),
            "expected footer to advertise retry for eligible task:\n{dump}"
        );
    }

    #[test]
    fn footer_advertises_stop_for_dispatched_task() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let backend = ratatui::backend::TestBackend::new(180, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            dump.contains("X: stop"),
            "expected footer to advertise stop for eligible task:\n{dump}"
        );
    }

    #[test]
    fn footer_advertises_review_only_for_awaiting_review_task() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_pending_review_for_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
            2,
        );
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let backend = ratatui::backend::TestBackend::new(180, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        let has_full_review_hint = dump.contains("a: approve")
            && dump.contains("d: reject")
            && dump.contains("c: changes");
        assert!(
            has_full_review_hint || dump.contains("a/d/c: review"),
            "expected footer to advertise review decisions for awaiting review task:\n{dump}"
        );
    }

    #[test]
    fn footer_omits_peek_keys_when_issue_overlay_open() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));
        view.set_open_issue_id_for_tests(Some("bd-epic.1".into()));

        let backend = ratatui::backend::TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            !dump.contains("s: stream peek"),
            "expected footer not to advertise stream peek while issue overlay is open:\n{dump}"
        );
        assert!(
            !dump.contains("S: jump to worker"),
            "expected footer not to advertise dashboard jump while issue overlay is open:\n{dump}"
        );
    }

    #[test]
    fn peek_overlay_title_uses_agent_name_when_lineage_knows_executor() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            dump.contains("stream: codex (t-12)"),
            "expected peek title to use the agent name:\n{dump}"
        );
        assert!(
            !dump.contains("stream: worker-session-1 (t-12)"),
            "expected peek title not to expose the raw executor id:\n{dump}"
        );
    }

    #[test]
    fn peek_overlay_title_shows_completed_badge_for_terminal_executor() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let mut lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        lineage.apply(&SpurEvent::now(SpurEventBody::ExecutorPhaseChanged {
            id: "worker-session-1".into(),
            phase: LifecycleState::Succeeded,
        }));
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();

        let dump = format!("{:?}", terminal.backend().buffer());
        assert!(
            dump.contains("[completed]"),
            "expected peek title to show completed badge:\n{dump}"
        );
    }

    #[test]
    fn peek_overlay_falls_back_to_fullscreen_below_60_cols() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let mut ws = crate::worker_streams::WorkerStreams::new();
        let backend = ratatui::backend::TestBackend::new(40, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn o_opens_source_work_item_from_plan_inspector() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic(&session_id);
        let lineage = ExecutorLineage::new();
        let synopsis = SessionSynopsisProjection::new();
        let ctx = ctx(&lineage, &projection, &synopsis);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key(KeyCode::Char('o')), &ctx);

        assert!(matches!(
            action,
            Some(Action::OpenIssueInBacklog { id }) if id == "bd-epic"
        ));
    }

    #[test]
    fn r_opens_start_resume_confirmation_for_active_plan() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('r'), &ctx);

        assert!(action.is_none(), "r should open confirmation first");
        assert!(matches!(
            view.confirm,
            Some(PlanInspectorConfirm::Start { ref plan_id }) if plan_id == "plan-1"
        ));
    }

    #[test]
    fn enter_from_start_resume_confirmation_resumes_plan() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_char('r'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::ResumePlan { plan_id }) if plan_id == "plan-1"
        ));
    }

    #[test]
    fn r_rejects_terminal_plan_with_hint() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_terminal_plan(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('r'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message })
                if message == "Cannot start: plan plan-1 is terminal"
        ));
    }

    #[test]
    fn esc_or_q_dismisses_start_resume_confirmation_without_action() {
        for close_key in [KeyCode::Esc, KeyCode::Char('q')] {
            let session_id = SessionId("brain-1".into());
            let projection = projection_with_epic(&session_id);
            let lineage = ExecutorLineage::new();
            let ctx = view_context_for_tests(&lineage, &projection);
            let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

            assert!(view.handle_key(key_char('r'), &ctx).is_none());
            let action = view.handle_key(key(close_key), &ctx);

            assert!(action.is_none());
            assert!(view.confirm.is_none());
        }
    }

    #[test]
    fn retryable_selected_task_opens_retry_confirmation() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "failed", Some("bd-epic.1"), 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('R'), &ctx);

        assert!(action.is_none(), "R should open confirmation first");
        assert!(matches!(
            view.confirm,
            Some(PlanInspectorConfirm::RetryTask {
                ref plan_id,
                ref task_id,
                ref task_name,
                ref issue_id,
                ref status,
                attempt,
                max_attempts,
                ref append_prompt,
                cursor,
            }) if plan_id == "plan-1"
                && task_id == "t-12"
                && task_name == "Stage A"
                && issue_id == "bd-epic.1"
                && status == "failed"
                && attempt == 1
                && max_attempts == 3
                && append_prompt.is_empty()
                && cursor == 0
        ));
    }

    #[test]
    fn stop_key_opens_confirm_for_dispatched_task() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_shift('X'), &ctx);

        assert!(action.is_none(), "X should open confirmation first");
        assert!(matches!(
            view.confirm,
            Some(PlanInspectorConfirm::StopTask {
                ref plan_id,
                ref task_id,
                ref task_name,
                ref delegation_id,
                ref status,
                ..
            }) if plan_id == "plan-1"
                && task_id == "t-12"
                && task_name == "Stage A"
                && delegation_id == "deleg-12"
                && status == "dispatched"
        ));
    }

    #[test]
    fn stop_key_rejected_for_non_dispatched_task() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "pending", Some("bd-epic.1"), 0, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_shift('X'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message }) if message == "Cannot stop: task t-12 is pending"
        ));
    }

    #[test]
    fn stop_key_rejected_without_delegation_id() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "dispatched", Some("bd-epic.1"), 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_shift('X'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message })
                if message == "Cannot stop: task t-12 has no delegation"
        ));
    }

    #[test]
    fn confirm_stop_emits_cancel_delegation() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_shift('X'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::CancelDelegation { delegation_id }) if delegation_id == "deleg-12"
        ));
    }

    // ── Operate-gate contracts (F1–F8) ─────────────────────────────────────
    // Audit findings locked as RED tests before the production fix:
    //   F1 stop → cancelled terminal (worker kill + Completion(Cancelled))
    //   F2 cancelled excluded from is_retryable (blocks stop-then-retry)
    //   F3 escalated_to_brain excluded from is_retryable
    //   F4 stop modal claims "Press R … with appended instructions"
    //   F5 stop confirm append_prompt is collected then discarded on confirm
    //   F6 retry is requeue-only (submit_plan_mutation retry_task), no inline spawn
    //   F7 retry mutation path does not fast_forward_reconciler
    //   F8 PlanCommandError ignored in PlanInspectorView (handled only in plan_browser)
    //
    // F6/F7 implementer note (bridge, not TUI-local — assert in spur-core):
    // After a successful RetryPlanTask / submit_plan_mutation(retry_task), call
    // `McpCallbackServer::fast_forward_reconciler()` so the reconciler leaves
    // idle backoff and re-dispatches. Preferred call site: success arm of
    // `handle_submit_plan_mutation` in crates/spur-core/src/server/handlers/plan.rs
    // (covers MCP + TUI `call_retry_plan_task`). Alternative: success path of
    // InteractiveInput::RetryPlanTask in interactive_loop.rs. Today neither
    // path notifies; requeue alone leaves the task open but undispatched.

    /// F1/F2: post-stop `cancelled` status must be retryable (stop-then-retry).
    #[test]
    fn post_stop_cancelled_status_opens_retry_confirmation() {
        let session_id = SessionId("brain-1".into());
        // Simulates projection after a successful stop path lands
        // Completion(Cancelled) → snapshot status "cancelled".
        let projection =
            projection_with_single_task_status(&session_id, "cancelled", Some("bd-epic.1"), 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('R'), &ctx);

        assert!(
            action.is_none(),
            "R on cancelled (post-stop) must open retry confirm, not flash; got {action:?}"
        );
        assert!(
            matches!(
                view.confirm,
                Some(PlanInspectorConfirm::RetryTask {
                    ref status,
                    ref issue_id,
                    ..
                }) if status == "cancelled" && issue_id == "bd-epic.1"
            ),
            "expected RetryTask confirm for cancelled task, got {:?}",
            view.confirm
        );
    }

    /// F3: escalated_to_brain is a recovery status and must open retry.
    #[test]
    fn escalated_to_brain_status_opens_retry_confirmation() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_single_task_status(
            &session_id,
            "escalated_to_brain",
            Some("bd-epic.1"),
            1,
            3,
        );
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('R'), &ctx);

        assert!(
            action.is_none(),
            "R on escalated_to_brain must open retry confirm; got {action:?}"
        );
        assert!(
            matches!(
                view.confirm,
                Some(PlanInspectorConfirm::RetryTask {
                    ref status,
                    ref issue_id,
                    ..
                }) if status == "escalated_to_brain" && issue_id == "bd-epic.1"
            ),
            "expected RetryTask confirm for escalated_to_brain, got {:?}",
            view.confirm
        );
    }

    /// F1/F2/F3: footer must advertise R for cancelled and escalated_to_brain.
    #[test]
    fn footer_advertises_retry_for_cancelled_and_escalated_statuses() {
        for status in ["cancelled", "escalated_to_brain"] {
            let session_id = SessionId(format!("brain-{status}"));
            let projection =
                projection_with_single_task_status(&session_id, status, Some("bd-epic.1"), 1, 3);
            let lineage = ExecutorLineage::new();
            let ctx = view_context_for_tests(&lineage, &projection);

            let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
            view.set_selected_task_id_for_tests(Some("t-12".into()));

            let backend = ratatui::backend::TestBackend::new(160, 24);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| {
                    <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
                })
                .unwrap();

            let dump = format!("{:?}", terminal.backend().buffer());
            assert!(
                dump.contains("R: retry"),
                "expected footer to advertise retry for status={status}:\n{dump}"
            );
        }
    }

    /// F4/F5: stop confirm must not show a dead append-prompt editor, and must
    /// not claim typed stop instructions will be applied (CancelDelegation has
    /// no prompt channel). Operators append instructions on the subsequent R
    /// retry modal after the task is cancelled.
    #[test]
    fn stop_confirm_does_not_offer_unwired_append_prompt() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_shift('X'), &ctx).is_none());
        assert!(matches!(
            view.confirm,
            Some(PlanInspectorConfirm::StopTask { .. })
        ));

        let backend = ratatui::backend::TestBackend::new(100, 28);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = buffer_text(terminal.backend().buffer());
        assert!(
            !dump.contains("Append:"),
            "stop confirm must not show an Append prompt editor unless it is wired through confirm; got:\n{dump}"
        );
        assert!(
            !dump.contains("with appended instructions"),
            "stop confirm must not claim stop-time typed instructions are applied; got:\n{dump}"
        );
    }

    #[test]
    fn stop_confirm_marks_retry_ineligible_at_attempt_limit() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_stoppable_task(&session_id, "deleg-12", 3, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_shift('X'), &ctx).is_none());

        let backend = ratatui::backend::TestBackend::new(100, 28);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let dump = buffer_text(terminal.backend().buffer());
        assert!(
            dump.contains("Retry unavailable: attempt limit reached (3/3)."),
            "stop confirm must explain that retry attempts are exhausted; got:\n{dump}"
        );
        assert!(
            !dump.contains("Press R afterward to retry."),
            "stop confirm must not promise retry after the attempt limit; got:\n{dump}"
        );
    }

    /// F8: PlanCommandError must surface in the inspector footer (or equivalent
    /// status strip), matching plan_browser's operator-visible error path.
    #[test]
    fn plan_command_error_surfaces_in_footer_status() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        <PlanInspectorView as View>::handle_spur_event(
            &mut view,
            &SpurEvent::now(SpurEventBody::PlanCommandError {
                operation: "RetryPlanTask".into(),
                plan_id: Some("plan-1".into()),
                error: "retry_task: issue is not retryable".into(),
            }),
            &ctx,
        );

        let backend = ratatui::backend::TestBackend::new(120, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                <PlanInspectorView as View>::render(&mut view, frame, frame.area(), &ctx);
            })
            .unwrap();

        let footer = buffer_row(terminal.backend().buffer(), 23);
        assert!(
            footer.contains("RetryPlanTask") || footer.contains("blocked"),
            "expected PlanCommandError to surface in footer status, got:\n{footer}"
        );
        assert!(
            footer.contains("not retryable") || footer.contains("issue is not"),
            "expected error detail in footer status, got:\n{footer}"
        );
    }

    #[test]
    fn pinned_inspector_ignores_plan_command_error_for_another_plan() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        <PlanInspectorView as View>::handle_spur_event(
            &mut view,
            &SpurEvent::now(SpurEventBody::PlanCommandError {
                operation: "RetryPlanTask".into(),
                plan_id: Some("plan-2".into()),
                error: "retry_task: issue is not retryable".into(),
            }),
            &ctx,
        );

        assert!(view.loop_event_status.is_none());
    }

    #[test]
    fn retry_confirmation_clears_prior_plan_command_error() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "rejected", Some("bd-epic.1"), 2, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        <PlanInspectorView as View>::handle_spur_event(
            &mut view,
            &SpurEvent::now(SpurEventBody::PlanCommandError {
                operation: "RetryPlanTask".into(),
                plan_id: Some("plan-1".into()),
                error: "retry_task: issue is not retryable".into(),
            }),
            &ctx,
        );
        assert!(view.loop_event_status.is_some());

        assert!(view.handle_key(key_char('R'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(matches!(action, Some(Action::RetryPlanTask { .. })));
        assert!(
            view.loop_event_status.is_none(),
            "successful local retry confirmation must clear the prior command error"
        );
    }

    #[test]
    fn enter_from_retry_confirmation_dispatches_retry_action() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "rejected", Some("bd-epic.1"), 2, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_char('R'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::RetryPlanTask {
                plan_id: Some(plan_id),
                issue_id,
                append_prompt: None,
            }) if plan_id == "plan-1" && issue_id == "bd-epic.1"
        ));
    }

    #[test]
    fn retry_confirmation_emits_typed_append_prompt() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "failed", Some("bd-epic.1"), 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_char('R'), &ctx).is_none());
        for ch in "add tests".chars() {
            assert!(view.handle_key(key_char(ch), &ctx).is_none());
        }
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::RetryPlanTask {
                plan_id: Some(plan_id),
                issue_id,
                append_prompt: Some(prompt),
            }) if plan_id == "plan-1"
                && issue_id == "bd-epic.1"
                && prompt == "add tests"
        ));
    }

    #[test]
    fn retry_missing_issue_id_flashes_hint() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_single_task_status(&session_id, "failed", None, 1, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('R'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message })
                if message == "Cannot retry: selected task has no linked issue"
        ));
    }

    #[test]
    fn retry_blocks_when_attempts_are_exhausted() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "error", Some("bd-epic.1"), 3, 3);
        let lineage = ExecutorLineage::new();
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('R'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message })
                if message == "Cannot retry: task t-12 is at attempt 3/3"
        ));
    }

    #[test]
    fn retry_blocks_non_retryable_running_and_pending_tasks() {
        for status in ["running", "pending"] {
            let session_id = SessionId(format!("brain-{status}"));
            let projection =
                projection_with_single_task_status(&session_id, status, Some("bd-epic.1"), 1, 3);
            let lineage = ExecutorLineage::new();
            let ctx = view_context_for_tests(&lineage, &projection);
            let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

            let action = view.handle_key(key_char('R'), &ctx);

            assert!(view.confirm.is_none());
            assert!(matches!(
                action,
                Some(Action::FlashHint { message })
                    if message == format!("Cannot retry: task t-12 is {status}")
            ));
        }
    }

    #[test]
    fn retry_modal_focus_does_not_leak_through_issue_detail_or_stream_peek() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_single_task_status(&session_id, "failed", Some("bd-epic.1"), 1, 3);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut issue_view = PlanInspectorView::new_for_plan(session_id.clone(), "plan-1".into());
        issue_view.set_selected_task_id_for_tests(Some("t-12".into()));
        issue_view.set_open_issue_id_for_tests(Some("bd-epic.1".into()));
        let issue_action = issue_view.handle_key_with_worker_streams(key_char('R'), &mut ws, &ctx);
        assert!(issue_action.is_none());
        assert!(issue_view.confirm.is_none());
        assert!(matches!(issue_view.mode(), PlanInspectorMode::Browse));

        let mut peek_view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        peek_view.enter_stream_peek("worker-session-1".into(), "t-12".into());
        let peek_action = peek_view.handle_key_with_worker_streams(key_char('R'), &mut ws, &ctx);
        assert!(peek_action.is_none());
        assert!(peek_view.confirm.is_none());
        assert!(matches!(
            peek_view.mode(),
            PlanInspectorMode::StreamPeek { .. }
        ));
    }

    #[test]
    fn reviewable_selected_task_opens_review_confirmation() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_pending_review_for_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
            2,
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('a'), &ctx);

        assert!(
            action.is_none(),
            "a should open confirmation before dispatch"
        );
        assert!(
            view.confirm.is_some(),
            "awaiting_review task should open a review confirmation"
        );
    }

    #[test]
    fn enter_from_approve_review_confirmation_dispatches_submit_review_action() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_pending_review_for_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
            2,
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_char('a'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::SubmitReview {
                executor_id,
                attempt_n: 2,
                decision: spur_core::ReviewDecision::Approve,
            }) if executor_id == "worker-session-1"
        ));
    }

    #[test]
    fn request_changes_review_confirmation_uses_task_feedback() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_pending_review_for_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
            2,
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        assert!(view.handle_key(key_char('c'), &ctx).is_none());
        let action = view.handle_key(key(KeyCode::Enter), &ctx);

        assert!(matches!(
            action,
            Some(Action::SubmitReview {
                executor_id,
                attempt_n: 2,
                decision: spur_core::ReviewDecision::Modify { note },
            }) if executor_id == "worker-session-1" && note == "please add tests"
        ));
    }

    #[test]
    fn review_missing_pending_review_flashes_hint() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_worker_for_task_and_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());

        let action = view.handle_key(key_char('a'), &ctx);

        assert!(view.confirm.is_none());
        assert!(matches!(
            action,
            Some(Action::FlashHint { message })
                if message == "Cannot review: task t-12 has no pending review"
        ));
    }

    #[test]
    fn review_modal_focus_does_not_leak_through_issue_detail_or_stream_peek() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_awaiting_review_task(
            &session_id,
            Some("please add tests"),
            Some("deleg-12"),
        );
        let lineage = lineage_with_pending_review_for_delegation(
            &session_id,
            "t-12",
            "worker-session-1",
            "deleg-12",
            2,
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut issue_view = PlanInspectorView::new_for_plan(session_id.clone(), "plan-1".into());
        issue_view.set_selected_task_id_for_tests(Some("t-12".into()));
        issue_view.set_open_issue_id_for_tests(Some("bd-epic.1".into()));
        let issue_action = issue_view.handle_key_with_worker_streams(key_char('a'), &mut ws, &ctx);
        assert!(issue_action.is_none());
        assert!(issue_view.confirm.is_none());
        assert!(matches!(issue_view.mode(), PlanInspectorMode::Browse));

        let mut peek_view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        peek_view.enter_stream_peek("worker-session-1".into(), "t-12".into());
        let peek_action = peek_view.handle_key_with_worker_streams(key_char('a'), &mut ws, &ctx);
        assert!(peek_action.is_none());
        assert!(peek_view.confirm.is_none());
        assert!(matches!(
            peek_view.mode(),
            PlanInspectorMode::StreamPeek { .. }
        ));
    }

    #[test]
    fn lowercase_s_enters_peek_when_task_has_worker() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");

        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let action = view.handle_key_with_worker_streams(key_char('s'), &mut ws, &ctx);

        assert!(matches!(view.mode(), PlanInspectorMode::StreamPeek { .. }));
        assert!(action.is_none());
    }

    #[test]
    fn s_does_not_open_peek_when_issue_overlay_is_active() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");

        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));
        view.set_open_issue_id_for_tests(Some("bd-epic.1".into()));

        let action = view.handle_key_with_worker_streams(key_char('s'), &mut ws, &ctx);

        assert!(matches!(view.mode(), PlanInspectorMode::Browse));
        assert!(action.is_none());
    }

    #[test]
    fn lowercase_s_flashes_hint_when_task_has_no_worker() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic(&session_id);
        let lineage = ExecutorLineage::new();

        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let action = view.handle_key_with_worker_streams(key_char('s'), &mut ws, &ctx);

        assert!(matches!(view.mode(), PlanInspectorMode::Browse));
        assert!(matches!(action, Some(Action::FlashHint { .. })));
    }

    #[test]
    fn shift_s_emits_focus_worker_action() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");

        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));

        let action = view.handle_key_with_worker_streams(key_shift('S'), &mut ws, &ctx);

        match action {
            Some(Action::FocusWorkerInDashboard { executor_id, tab }) => {
                assert_eq!(executor_id, "worker-session-1");
                assert_eq!(tab, crate::components::detail_pane::DetailTab::Stream);
            }
            other => panic!("expected FocusWorkerInDashboard, got {:?}", other),
        }
    }

    #[test]
    fn esc_leaves_peek() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let action = view.handle_key_with_worker_streams(key_code(KeyCode::Esc), &mut ws, &ctx);

        assert!(matches!(view.mode(), PlanInspectorMode::Browse));
        assert!(action.is_none());
    }

    #[test]
    fn j_scrolls_peek_without_leaking_to_task_list() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        let initial_selection = view.selected_task_id_for_tests();
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        if let Some(state) = view.peek_state_mut_for_tests() {
            state.scroll_offset = 10;
            state.is_following = false;
        }

        let _ = view.handle_key_with_worker_streams(key_char('k'), &mut ws, &ctx);

        if let PlanInspectorMode::StreamPeek { state, .. } = view.mode() {
            assert_eq!(state.scroll_offset, 9);
        }
        assert_eq!(view.selected_task_id_for_tests(), initial_selection);
    }

    #[test]
    fn f_toggles_follow_in_peek() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_epic_and_worker(&session_id);
        let lineage = lineage_with_worker_for_task(&session_id, "t-12", "worker-session-1");
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let initial = match view.mode() {
            PlanInspectorMode::StreamPeek { state, .. } => state.is_following,
            _ => panic!(),
        };

        let _ = view.handle_key_with_worker_streams(key_char('f'), &mut ws, &ctx);

        if let PlanInspectorMode::StreamPeek { state, .. } = view.mode() {
            assert_eq!(state.is_following, !initial);
        }
    }

    #[test]
    fn peek_auto_closes_when_selected_task_changes() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_two_running_tasks(&session_id);
        let lineage = lineage_with_two_workers(&session_id);
        let _ctx = view_context_for_tests(&lineage, &projection);

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        view.set_selected_task_id_for_tests(Some("t-13".into()));

        assert_eq!(view.selected_task_id_for_tests(), Some("t-13".into()));
        assert!(matches!(view.mode(), PlanInspectorMode::Browse));
    }

    #[test]
    fn peek_auto_closes_when_task_delegation_swaps_executor() {
        let session_id = SessionId("brain-1".into());
        let projection =
            projection_with_epic_and_worker_for_delegation(&session_id, "deleg-retry-1");
        let lineage = lineage_with_worker_for_task_and_delegation(
            &session_id,
            "t-12",
            "worker-session-2",
            "deleg-retry-1",
        );
        let ctx = view_context_for_tests(&lineage, &projection);
        let mut ws = crate::worker_streams::WorkerStreams::new();

        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        view.set_selected_task_id_for_tests(Some("t-12".into()));
        view.enter_stream_peek("worker-session-1".into(), "t-12".into());

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                view.render_with_worker_streams(frame, frame.area(), &mut ws, &ctx);
            })
            .unwrap();

        assert!(matches!(view.mode(), PlanInspectorMode::Browse));
    }

    #[test]
    fn b_cycles_selection_through_selected_task_blockers() {
        let session_id = SessionId("brain-1".into());
        let projection = projection_with_blocker_cycle(&session_id);
        let lineage = ExecutorLineage::new();
        let synopsis = SessionSynopsisProjection::new();
        let ctx = ctx(&lineage, &projection, &synopsis);
        let mut view = PlanInspectorView::new_for_plan(session_id, "plan-1".into());
        let plan = projection.plan("plan-1").unwrap();

        view.set_selected_task_id(Some("ui".into()));
        assert!(view.handle_key(key(KeyCode::Char('b')), &ctx).is_none());
        assert_eq!(
            view.selected_task(plan).map(|task| task.task_id.as_str()),
            Some("lint")
        );

        assert!(view.handle_key(key(KeyCode::Char('b')), &ctx).is_none());
        assert_eq!(
            view.selected_task(plan).map(|task| task.task_id.as_str()),
            Some("api")
        );
    }

    #[test]
    fn b_flashes_hint_for_external_blocker() {
        let session_id = SessionId("brain-1".into());
        let mut projection = projection_with_blocker_cycle(&session_id);
        projection.apply(&SpurEvent::now(SpurEventBody::PlanSnapshotUpdated {
            session_id: session_id.clone(),
            snapshot: Box::new(PlanSnapshot {
                plan_id: "external-plan".into(),
                epic_id: None,
                status: "running".into(),
                progress: "0/1 done".into(),
                next_action: "wait for external issue".into(),
                ready_to_merge: false,
                counts: PlanSnapshotCounts {
                    pending: 1,
                    ..Default::default()
                },
                tasks: vec![PlanSnapshotTask {
                    task_id: "ui".into(),
                    task_name: "ui".into(),
                    agent: "codex".into(),
                    issue_id: Some("bd-ui".into()),
                    issue_title: None,
                    status: "pending".into(),
                    attempt: 0,
                    max_attempts: 3,
                    depends_on: Vec::new(),
                    blocked_by: vec!["bd-external".into()],
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
                }],
                owner_brain_session_id: Some(session_id.0.clone()),
                owner_token: None,
                owner_acquired_at: None,
            }),
        }));
        let lineage = ExecutorLineage::new();
        let synopsis = SessionSynopsisProjection::new();
        let ctx = ctx(&lineage, &projection, &synopsis);
        let mut view = PlanInspectorView::new_for_plan(session_id, "external-plan".into());

        assert!(matches!(
            view.handle_key(key(KeyCode::Char('b')), &ctx),
            Some(Action::FlashHint { message }) if message.contains("bd-external")
        ));
    }
}
