//! Level-triggered reconciler for beads-backed plans.
//!
//! Ticks on an adaptive cadence: fast when there is activity, backing off
//! toward an idle ceiling when there is not. When constructed with a
//! `ReconcilerDispatchCtx`, the reconciler persists dispatch intent and
//! enqueues ACP work for ready persisted tasks.
//!
//! Primary engine: `bv --robot-triage` via BvAdapter (see plan
//! addendum II in docs/superpowers/plans/2026-04-20-adaptive-plan-repair-v0a.md
//! for the rationale — upstream AGENTS.md designates bv as the canonical
//! pick-next-work surface). Fallback: `br ready` via BeadsAdvanced when bv
//! errors. Ready observation enforces the epic activation guard: a plan must
//! carry `spur:plan-complete` and must not carry `spur:plan-pending`.
//!
//! # Spawn wiring
//!
//! In v0c the reconciler is wired into `server.rs` startup with a live
//! `ReconcilerDispatchCtx`, so persisted plans are reclaimed and dispatched by
//! the same loop that owns completion writeback.
//!
//! # Submodule layout
//!
//! | Module | Responsibility |
//! |---|---|
//! | `leases` | Sweep expired dispatch leases and reclaim orphaned dispatches. |
//! | `conflict` | Detect and persist overlay conflicts (setup and predispatch). |
//! | `ready` | Observe ready tasks from beads backend with plan-activation guards. |
//! | `base_spec` | Git helpers and base-spec construction for dispatch overlays. |
//! | `guards` | Plan activation guards (`plan_allows_dispatch`, `plan_allows_writes`). |
//! | `terminal` | Reconcile terminal epics: completion audits, auto-merge, auto-PR. |
//! | `tests` | Unit tests, mocks, and shared fixtures for the reconciler. |

mod hygiene;
mod leases;

mod ownership;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::server::AbortableTaskTracker;
use futures::future::BoxFuture;
use tokio::sync::Notify;

use crate::plan::outcomes::{
    DispatchOutcome, NoReadyReason, OutcomeLogDecision, OutcomeStore, SkipReason, StuckTask,
};

const COMPLETION_PROJECTION_TIMEOUT: Duration = Duration::from_secs(10);

mod ready;

mod reviews;

mod conflict;
pub(crate) use conflict::persist_observed_write_conflict;

mod base_spec;

use base_spec::plan_dispatch_base_spec;

mod guards;

mod topology;

pub use terminal::ReconcilerAutomation;
mod terminal;

pub(crate) fn beads_journal_path(repo_root: &std::path::Path) -> std::path::PathBuf {
    repo_root.join(".beads").join("journal")
}

pub(crate) async fn monitor_journal_appends(path: std::path::PathBuf, notify: Arc<Notify>) {
    let mut last_len = match tokio::fs::metadata(&path).await {
        Ok(meta) => meta.len(),
        Err(error) => {
            tracing::debug!(
                %error,
                ?path,
                "journal metadata unavailable at startup; disabling poller"
            );
            return;
        }
    };
    let keep_polling = true;
    while keep_polling {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let next_len = match tokio::fs::metadata(&path).await {
            Ok(meta) => meta.len(),
            Err(error) => {
                tracing::debug!(%error, ?path, "journal metadata unavailable; retrying");
                continue;
            }
        };
        if next_len > last_len {
            last_len = next_len;
            notify.notify_one();
        } else {
            last_len = next_len;
        }
    }
}

#[derive(Clone)]
pub(crate) struct CompletionCollectorLogContext {
    pub(crate) plan_id: String,
    pub(crate) task_id: String,
    pub(crate) delegation_id: String,
    pub(crate) brain_session_id: String,
    pub(crate) attempt: u32,
}

pub(crate) fn spawn_completion_collector<F>(
    dispatch: &dyn ReconcilerDispatch,
    context: CompletionCollectorLogContext,
    future: F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    dispatch.track_task(Box::pin(async move {
        use futures::FutureExt;

        match std::panic::AssertUnwindSafe(future).catch_unwind().await {
            Ok(()) => {}
            Err(payload) => {
                log_completion_collector_panic(&context, payload.as_ref());
                std::panic::resume_unwind(payload);
            }
        }
    }));
}

fn log_completion_collector_panic(
    context: &CompletionCollectorLogContext,
    payload: &(dyn std::any::Any + Send),
) {
    let panic_message = panic_payload_message(payload);
    let backtrace = std::backtrace::Backtrace::force_capture();
    tracing::error!(
        target: "spur.reconciler.completion_collector",
        plan_id = %context.plan_id,
        task_id = %context.task_id,
        delegation_id = %context.delegation_id,
        brain_session_id = %context.brain_session_id,
        attempt = context.attempt,
        panic_message = %panic_message,
        backtrace = %backtrace,
        "completion collector panicked"
    );
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

struct CompletionProjectionLogContext<'a> {
    plan_id: &'a str,
    task_id: &'a str,
    delegation_id: &'a str,
    brain_session_id: &'a spur_acp::BrainSessionId,
    attempt: u32,
}

async fn project_completion_snapshot_and_deliver<F>(
    clock: &dyn Clock,
    projection: F,
    outcomes: &Arc<tokio::sync::Mutex<OutcomeStore>>,
    event_sink: Option<&dyn spur_mcp::events::McpEventSink>,
    continuation_ctx: &crate::plan::continuation::DetachedContinuationCtx,
    deferred: Option<crate::plan::DeferredCompletionPush>,
    context: CompletionProjectionLogContext<'_>,
) where
    F: Future<Output = anyhow::Result<crate::plan::PlanState>>,
{
    tokio::pin!(projection);
    match tokio::select! {
        result = &mut projection => Ok(result),
        _ = clock.sleep(COMPLETION_PROJECTION_TIMEOUT) => Err(()),
    } {
        Ok(Ok(projected)) => {
            tracing::info!(
                target: "spur.reconciler.completion_collector",
                plan_id = %context.plan_id,
                task_id = %context.task_id,
                delegation_id = %context.delegation_id,
                brain_session_id = %context.brain_session_id,
                attempt = context.attempt,
                projected_task_count = projected.tasks.len(),
                "stage_completed_project"
            );
            tracing::info!(
                target: "spur.reconciler.completion_collector",
                plan_id = %context.plan_id,
                task_id = %context.task_id,
                delegation_id = %context.delegation_id,
                brain_session_id = %context.brain_session_id,
                attempt = context.attempt,
                "stage_entered_prune"
            );
            prune_projected_terminal_task_outcomes(outcomes, context.plan_id, &projected.tasks)
                .await;
            tracing::info!(
                target: "spur.reconciler.completion_collector",
                plan_id = %context.plan_id,
                task_id = %context.task_id,
                delegation_id = %context.delegation_id,
                brain_session_id = %context.brain_session_id,
                attempt = context.attempt,
                "stage_completed_prune"
            );
            if let Some(sink) = event_sink {
                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    plan_id = %context.plan_id,
                    task_id = %context.task_id,
                    delegation_id = %context.delegation_id,
                    brain_session_id = %context.brain_session_id,
                    attempt = context.attempt,
                    "stage_entered_snapshot"
                );
                crate::plan::snapshot::emit_plan_snapshot(Some(sink), &projected);
                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    plan_id = %context.plan_id,
                    task_id = %context.task_id,
                    delegation_id = %context.delegation_id,
                    brain_session_id = %context.brain_session_id,
                    attempt = context.attempt,
                    "stage_completed_snapshot"
                );
            }
        }
        Ok(Err(error)) => tracing::warn!(
            plan_id = %context.plan_id,
            task_id = %context.task_id,
            "failed to project plan snapshot after completion: {error}"
        ),
        Err(_) => {
            let timeout_ms = COMPLETION_PROJECTION_TIMEOUT.as_millis() as u64;
            tracing::error!(
                target: "spur.reconciler.completion_collector",
                plan_id = %context.plan_id,
                task_id = %context.task_id,
                delegation_id = %context.delegation_id,
                brain_session_id = %context.brain_session_id,
                attempt = context.attempt,
                timeout_ms,
                "project stage timed out — abandoning snapshot for this completion (event delivered via deferred push when present)"
            );
            tracing::info!(
                target: "spur.reconciler.completion_collector",
                plan_id = %context.plan_id,
                task_id = %context.task_id,
                delegation_id = %context.delegation_id,
                brain_session_id = %context.brain_session_id,
                attempt = context.attempt,
                timeout_ms,
                "stage_completed_project_timeout"
            );
            if let Some(deferred) = deferred {
                deferred.deliver(event_sink, continuation_ctx).await;
                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    plan_id = %context.plan_id,
                    task_id = %context.task_id,
                    delegation_id = %context.delegation_id,
                    brain_session_id = %context.brain_session_id,
                    attempt = context.attempt,
                    timeout_ms,
                    "stage_delivered_on_project_timeout"
                );
            } else {
                tracing::warn!(
                    target: "spur.reconciler.completion_collector",
                    plan_id = %context.plan_id,
                    task_id = %context.task_id,
                    delegation_id = %context.delegation_id,
                    brain_session_id = %context.brain_session_id,
                    attempt = context.attempt,
                    timeout_ms,
                    "completion lost — no deferred queue to requeue into"
                );
            }
            return;
        }
    }

    if let Some(deferred) = deferred {
        tracing::info!(
            target: "spur.reconciler.completion_collector",
            plan_id = %context.plan_id,
            task_id = %context.task_id,
            delegation_id = %context.delegation_id,
            brain_session_id = %context.brain_session_id,
            attempt = context.attempt,
            "stage_entered_deliver"
        );
        deferred.deliver(event_sink, continuation_ctx).await;
        tracing::info!(
            target: "spur.reconciler.completion_collector",
            plan_id = %context.plan_id,
            task_id = %context.task_id,
            delegation_id = %context.delegation_id,
            brain_session_id = %context.brain_session_id,
            attempt = context.attempt,
            "stage_completed_deliver"
        );
    }
}

fn issue_to_summary(issue: spur_pm::Issue) -> spur_pm::IssueSummary {
    spur_pm::IssueSummary {
        id: issue.id.clone(),
        source: issue.source,
        title: issue.title,
        status: issue.status,
        labels: issue.labels,
        url: issue.url,
        priority: issue.priority,
        issue_type: issue.issue_type,
        assignee: issue.assignee,
        description: Some(issue.body).filter(|b| !b.trim().is_empty()),
    }
}

fn task_id_from_labels_or_issue(labels: &[String], issue_id: &str) -> String {
    labels
        .iter()
        .find_map(|label| crate::plan::labels::parse_plan_task_id(label))
        .unwrap_or_else(|| issue_id.to_string())
}

fn delegation_label_value(label: &str) -> Option<&str> {
    crate::plan::labels::parse_delegation_id(label).or_else(|| label.strip_prefix("delegation-id:"))
}

fn is_ready_for_review_label(label: &str) -> bool {
    label == crate::plan::labels::READY_FOR_REVIEW || label == "ready-for-review"
}

fn is_review_rejected_label(label: &str) -> bool {
    label == crate::plan::labels::REVIEW_REJECTED || label == "review-rejected"
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LabelDriftEvent {
    label_family: &'static str,
    direction: &'static str,
}

struct LabelReconcileBuffers<'a> {
    add_labels: &'a mut Vec<String>,
    remove_labels: &'a mut Vec<String>,
    drift_events: &'a mut Vec<LabelDriftEvent>,
}

fn emit_label_index_drift(event: &LabelDriftEvent) {
    tracing::info!(
        label_kind = event.label_family,
        direction = event.direction,
        "label_index_drift"
    );
}

fn expected_plan_id_from_audits(
    audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
) -> Option<String> {
    use crate::plan::audit_sentinel::AuditSentinelKind;
    for audit in audits.iter().rev() {
        match audit {
            AuditSentinelKind::PlanSubmit { plan_id, .. }
            | AuditSentinelKind::EpicCompletion { plan_id, .. }
            | AuditSentinelKind::TaskTransition { plan_id, .. }
            | AuditSentinelKind::EscalationRequested { plan_id, .. } => {
                return Some(plan_id.clone());
            }
            _ => {}
        }
    }
    None
}

fn expected_plan_task_id_from_audits(
    audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
) -> Option<String> {
    if let Some((task_id, _, _, _, _, _, _, _)) = crate::plan::projector::latest_task_spec(audits) {
        return Some(task_id);
    }
    use crate::plan::audit_sentinel::AuditSentinelKind;
    for audit in audits.iter().rev() {
        match audit {
            AuditSentinelKind::TaskTransition { task_id, .. }
            | AuditSentinelKind::EscalationRequested { task_id, .. } => {
                return Some(task_id.clone());
            }
            _ => {}
        }
    }
    None
}

fn expected_agent_from_audits(
    audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
) -> Option<String> {
    let (_, agent, _) = crate::plan::projector::latest_extended_task_spec(audits);
    agent
}

fn expected_plan_complete_from_audits(
    issue_id: &str,
    audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
) -> Option<bool> {
    for audit in audits.iter().rev() {
        if let crate::plan::audit_sentinel::AuditSentinelKind::PlanSubmit {
            epic_issue_id, ..
        } = audit
        {
            return Some(epic_issue_id == issue_id);
        }
    }
    None
}

fn expected_plan_pending_from_audits(
    _issue_id: &str,
    audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
) -> Option<bool> {
    // `spur:plan-pending` exists only as a pre-audit transient while
    // `build_epic_subgraph` is still creating children/dependency edges.
    // The label is removed before `PlanSubmit` is emitted, so once any
    // `PlanSubmit` appears in audits this label MUST be absent.
    // `Some(true)` is structurally unreachable.
    for audit in audits.iter().rev() {
        if matches!(
            audit,
            crate::plan::audit_sentinel::AuditSentinelKind::PlanSubmit { .. }
        ) {
            return Some(false);
        }
    }
    None
}

fn reconcile_singleton_label(
    label_family: &'static str,
    expected: Option<String>,
    existing: Vec<String>,
    add_labels: &mut Vec<String>,
    remove_labels: &mut Vec<String>,
    drift_events: &mut Vec<LabelDriftEvent>,
) {
    match expected {
        Some(expected) => {
            let has_expected = existing.iter().any(|label| label == &expected);
            if !has_expected {
                drift_events.push(LabelDriftEvent {
                    label_family,
                    direction: if existing.is_empty() {
                        "missing"
                    } else {
                        "mismatched"
                    },
                });
                add_labels.push(expected.clone());
            }
            for existing_label in existing {
                if existing_label != expected {
                    drift_events.push(LabelDriftEvent {
                        label_family,
                        direction: "mismatched",
                    });
                    remove_labels.push(existing_label);
                }
            }
        }
        None => {
            for existing_label in existing {
                drift_events.push(LabelDriftEvent {
                    label_family,
                    direction: "stale",
                });
                remove_labels.push(existing_label);
            }
        }
    }
}

fn reconcile_presence_label(
    label_family: &'static str,
    canonical_label: &'static str,
    is_family_label: impl Fn(&str) -> bool,
    expected_present: bool,
    issue_labels: &[String],
    buffers: &mut LabelReconcileBuffers<'_>,
) {
    let has_present = issue_labels.iter().any(|label| is_family_label(label));
    if expected_present && !has_present {
        buffers.drift_events.push(LabelDriftEvent {
            label_family,
            direction: "missing",
        });
        buffers.add_labels.push(canonical_label.to_string());
    }
    if !expected_present {
        for label in issue_labels {
            if is_family_label(label) {
                buffers.drift_events.push(LabelDriftEvent {
                    label_family,
                    direction: "stale",
                });
                buffers.remove_labels.push(label.clone());
            }
        }
    }
}

fn unresolved_blocker_issue_ids(
    projected: &crate::plan::PlanState,
    task: &crate::plan::PlanTaskEntry,
) -> Vec<String> {
    let by_task_id = projected
        .tasks
        .iter()
        .map(|entry| (entry.spec.task_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let by_issue_id = projected
        .tasks
        .iter()
        .filter_map(|entry| {
            entry
                .spec
                .issue_id
                .as_deref()
                .map(|issue_id| (issue_id, entry))
        })
        .collect::<HashMap<_, _>>();

    let mut blockers = task
        .spec
        .depends_on
        .iter()
        .filter_map(|dependency| {
            let dependency_entry = by_task_id
                .get(dependency.as_str())
                .or_else(|| by_issue_id.get(dependency.as_str()));
            match dependency_entry {
                Some(entry)
                    if matches!(
                        entry.status,
                        crate::plan::PlanTaskStatus::Approved { .. }
                            | crate::plan::PlanTaskStatus::Cancelled { .. }
                            | crate::plan::PlanTaskStatus::Superseded { .. }
                    ) =>
                {
                    None
                }
                Some(entry) => Some(
                    entry
                        .spec
                        .issue_id
                        .clone()
                        .unwrap_or_else(|| entry.spec.task_id.clone()),
                ),
                None => Some(dependency.clone()),
            }
        })
        .collect::<Vec<_>>();
    blockers.sort();
    blockers.dedup();
    blockers
}

fn prior_branch_for_reuse(task: &crate::plan::PlanTaskEntry) -> Option<String> {
    task.history.last().and_then(|last| {
        if last.reuse_prior_worktree == Some(true) {
            last.worker_branch.clone()
        } else {
            None
        }
    })
}

async fn projected_plan_for_ready<Fut>(
    hydrated_plan_state: Option<Arc<crate::plan::PlanState>>,
    project: Fut,
) -> anyhow::Result<Arc<crate::plan::PlanState>>
where
    Fut: Future<Output = anyhow::Result<crate::plan::PlanState>>,
{
    if let Some(plan_state) = hydrated_plan_state {
        return Ok(plan_state);
    }

    Ok(Arc::new(project.await?))
}

fn report_only_dispatch_state(
    epic_labels: &[String],
    task_labels: &[String],
    epic_id: Option<&str>,
) -> Option<PlanDispatchState> {
    let autonomy = epic_labels
        .iter()
        .filter_map(|label| crate::plan::labels::parse_autonomy(label))
        .max();
    if autonomy == Some(crate::plan::labels::AutonomyLevel::L1)
        && !task_labels
            .iter()
            .any(|label| label == crate::plan::labels::LOOP_TRIAGE_TASK)
    {
        return Some(PlanDispatchState::ReportOnly {
            epic_id: epic_id.unwrap_or_default().to_string(),
        });
    }
    None
}

pub(crate) async fn prune_projected_terminal_task_outcomes(
    outcomes: &Arc<tokio::sync::Mutex<OutcomeStore>>,
    plan_id: &str,
    tasks: &[crate::plan::PlanTaskEntry],
) {
    let terminal_task_ids = tasks
        .iter()
        .filter(|task| task.status.is_terminal())
        .map(|task| task.spec.task_id.clone())
        .collect::<Vec<_>>();
    if terminal_task_ids.is_empty() {
        return;
    }

    let mut outcomes = outcomes.lock().await;
    for task_id in terminal_task_ids {
        outcomes.prune_task(plan_id, &task_id);
    }
}

struct SetupConflictContinuation<'a> {
    plan: &'a crate::plan::PlanState,
    repo_root: &'a std::path::Path,
    task_id: &'a str,
    delegation_id: &'a str,
    attempt: u32,
    dep_task_id: &'a str,
    files: &'a [String],
    summary: &'static str,
    event_sink: Option<&'a dyn spur_mcp::events::McpEventSink>,
    continuation_ctx: &'a crate::plan::continuation::DetachedContinuationCtx,
}

async fn emit_setup_conflict_continuation(input: SetupConflictContinuation<'_>) {
    let topology = match topology::compile_setup_conflict_topology(
        input.plan,
        input.repo_root,
        input.task_id,
        input.dep_task_id,
        input.files,
    )
    .await
    {
        Ok(t) => Some(t),
        Err(error) => {
            tracing::warn!(
                plan_id = %input.plan.plan_id,
                task_id = %input.task_id,
                "setup conflict topology compilation failed: {error}"
            );
            None
        }
    };

    let payload = spur_acp::domain::continuation::ContinuationPayload {
        resolved_config: None,
        status: spur_acp::domain::delegation::DelegationStatus::SetupFailed {
            error: spur_acp::domain::delegation::AttemptSetupError::OverlayConflict {
                source_task_id: input.dep_task_id.to_string(),
                files: input.files.to_vec(),
            },
        },
        summary: Some(input.summary.into()),
        diff_summary: None,
        worker_branch: None,
        artifact_ref: None,
        estimated_cost_micros: None,
        artifact_id: None,
        fetch_hint: None,
        base_hint: None,
        setup_conflict_topology: topology.clone(),
    };

    let cont = spur_acp::domain::continuation::BrainContinuation {
        delegation_id: spur_acp::DelegationId(input.delegation_id.to_string()),
        attempt: input.attempt,
        brain_session: input.plan.brain_session_id.as_session_id().clone(),
        source: spur_acp::domain::continuation::ContinuationSource::PlanTaskBlockedOnSetupConflict,
        payload,
        created_at_wall: chrono::Utc::now(),
        created_at_mono: std::time::Instant::now(),
    };

    if let Some(sink) = input.event_sink {
        sink.emit(spur_acp::SpurEventBody::PlanTaskBlockedOnSetupConflict {
            plan_id: input.plan.plan_id.clone(),
            task_id: input.task_id.to_string(),
            delegation_id: input.delegation_id.to_string(),
            dep_task_id: input.dep_task_id.to_string(),
            files: input.files.to_vec(),
            topology,
        });
    }

    (input.continuation_ctx.on_complete)(cont, input.delegation_id.to_string()).await;
}

#[derive(Clone)]
pub struct ReconcilerDispatchCtx {
    pub delegation_tx: tokio::sync::mpsc::Sender<crate::DelegationRequest>,
    pub task_tracker: AbortableTaskTracker,
    pub brain_session_id: spur_acp::BrainSessionId,
    pub event_sink: Option<Arc<dyn spur_mcp::events::McpEventSink>>,
    pub materializer: Arc<crate::outcome_materializer::OutcomeMaterializer>,
    pub continuation_ctx: Arc<crate::plan::continuation::DetachedContinuationCtx>,
}

#[async_trait::async_trait]
pub trait ReconcilerDispatch: Send + Sync {
    async fn send_delegation(&self, request: crate::DelegationRequest) -> anyhow::Result<()>;

    fn track_task(&self, fut: BoxFuture<'static, ()>);

    fn event_sink(&self) -> Option<&Arc<dyn spur_mcp::events::McpEventSink>>;

    fn materializer(&self) -> &Arc<crate::outcome_materializer::OutcomeMaterializer>;

    fn continuation_ctx(&self) -> &Arc<crate::plan::continuation::DetachedContinuationCtx>;

    fn brain_session_id(&self) -> &spur_acp::BrainSessionId;
}

#[async_trait::async_trait]
impl ReconcilerDispatch for ReconcilerDispatchCtx {
    async fn send_delegation(&self, request: crate::DelegationRequest) -> anyhow::Result<()> {
        self.delegation_tx
            .send(request)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    fn track_task(&self, fut: BoxFuture<'static, ()>) {
        let _ = self.task_tracker.spawn(fut);
    }

    fn event_sink(&self) -> Option<&Arc<dyn spur_mcp::events::McpEventSink>> {
        self.event_sink.as_ref()
    }

    fn materializer(&self) -> &Arc<crate::outcome_materializer::OutcomeMaterializer> {
        &self.materializer
    }

    fn continuation_ctx(&self) -> &Arc<crate::plan::continuation::DetachedContinuationCtx> {
        &self.continuation_ctx
    }

    fn brain_session_id(&self) -> &spur_acp::BrainSessionId {
        &self.brain_session_id
    }
}

impl ReconcilerDispatchCtx {
    pub fn into_dispatch(self) -> Arc<dyn ReconcilerDispatch> {
        Arc::new(self)
    }
}

#[derive(Debug, Clone)]
pub struct HydratedReady {
    pub summary: spur_pm::IssueSummary,
    pub plan_state: Option<Arc<crate::plan::PlanState>>,
}

/// Strategy for the pre-dispatch overlay dry-run. Production uses `Real`;
/// tests inject deterministic outcomes without touching git.
#[derive(Debug, Clone, Default)]
pub enum PreviewStrategy {
    #[default]
    Real,
    AlwaysClean,
    AlwaysConflict {
        dep_task_id: String,
        files: Vec<String>,
    },
}

pub struct ReconcilerConfig {
    pub base_interval: Duration,
    pub idle_ceiling: Duration,
    pub backoff_factor: u32,
    pub dispatch_lease_duration: Duration,
    pub label_only_dispatch_grace: Duration,
    pub repo_root: PathBuf,
    pub predispatch_preview: PreviewStrategy,
    pub loops_enabled: bool,
    pub pause_all_loops: bool,
    pub loop_sweep_scope: crate::plan::loops::LoopSweepScope,
    pub plan_scope: PlanScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanScope {
    BrainOwned,
    SystemL3Only,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_secs(3),
            idle_ceiling: Duration::from_secs(30),
            backoff_factor: 2,
            dispatch_lease_duration: Duration::from_secs(600),
            label_only_dispatch_grace: Duration::from_secs(30),
            repo_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            predispatch_preview: PreviewStrategy::Real,
            loops_enabled: true,
            pause_all_loops: false,
            loop_sweep_scope: crate::plan::loops::LoopSweepScope::All,
            plan_scope: PlanScope::BrainOwned,
        }
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;

    fn sleep<'a>(&'a self, duration: Duration) -> BoxFuture<'a, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanDispatchState {
    Allowed,
    PlanMissingCompleteEpic,
    EpicNotOpen { epic_id: String },
    PlanHasPendingEpic { epic_id: String },
    PlanOwnedByAnotherBrain { epic_id: String, owner: String },
    LoopsPaused { epic_id: String, scope: String },
    ReportOnly { epic_id: String },
    BudgetExhausted { spent_micros: u64, cap_micros: u64 },
}

impl PlanDispatchState {
    fn skip_reason(&self) -> Option<SkipReason> {
        match self {
            Self::Allowed => None,
            Self::PlanMissingCompleteEpic => Some(SkipReason::PlanMissingCompleteEpic),
            Self::EpicNotOpen { .. } => Some(SkipReason::EpicNotOpen),
            Self::PlanHasPendingEpic { .. } => Some(SkipReason::PlanHasPendingEpic),
            Self::PlanOwnedByAnotherBrain { owner, .. } => {
                Some(SkipReason::PlanOwnedByAnotherBrain {
                    owner: owner.clone(),
                })
            }
            Self::LoopsPaused { scope, .. } => Some(SkipReason::LoopsPaused {
                scope: scope.clone(),
            }),
            Self::ReportOnly { .. } => Some(SkipReason::ReportOnly),
            Self::BudgetExhausted {
                spent_micros,
                cap_micros,
            } => Some(SkipReason::BudgetExhausted {
                spent_micros: *spent_micros,
                cap_micros: *cap_micros,
            }),
        }
    }
}

pub struct Reconciler {
    pub(super) config: ReconcilerConfig,
    pub(super) pm: Arc<dyn crate::plan::PmLike>,
    pub(super) fast_forward: Arc<Notify>,
    pub(super) dispatch: Option<Arc<dyn ReconcilerDispatch>>,
    pub(super) plan_id: Option<String>,
    pub(super) auto_merge_approved_plans: bool,
    pub(super) automation: Option<Arc<dyn ReconcilerAutomation>>,
    pub(super) journal_wake: Option<Arc<Notify>>,
    pub(super) feature_gate: Arc<spur_license::FeatureGate>,
    pub(super) outcomes: Arc<tokio::sync::Mutex<OutcomeStore>>,
    pub(super) clock: Arc<dyn Clock>,
    hygiene: tokio::sync::Mutex<hygiene::HygieneState>,
}

impl Reconciler {
    pub(super) fn scoped_epic_labels(&self) -> Vec<String> {
        let mut labels = vec![crate::plan::labels::PLAN_COMPLETE.to_string()];
        match self.config.plan_scope {
            PlanScope::BrainOwned => {
                if let Some(dispatch) = self.dispatch.as_ref() {
                    labels.push(crate::plan::labels::plan_owner(
                        &dispatch.brain_session_id().as_session_id().0,
                    ));
                }
            }
            PlanScope::SystemL3Only => {
                labels.push(crate::plan::labels::plan_owner(
                    crate::plan::loops::LOOP_RUNTIME_OWNER_ID,
                ));
                labels.push(format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX));
            }
        }
        labels
    }

    pub(super) async fn scoped_plan_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut labels = self.scoped_epic_labels();
        if let Some(plan_id) = self.plan_id.as_deref() {
            labels.push(crate::plan::labels::plan_id(plan_id));
        }
        Ok(self
            .pm
            .list_issues(spur_pm::IssueFilter {
                labels,
                issue_type: Some("epic".to_string()),
                limit: Some(10_000),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter_map(|epic| {
                epic.labels
                    .iter()
                    .find_map(|label| crate::plan::labels::parse_plan_id(label))
                    .map(str::to_string)
            })
            .collect())
    }

    pub fn new(
        config: ReconcilerConfig,
        pm: Arc<spur_pm::PmService>,
        fast_forward: Arc<Notify>,
        dispatch: Option<ReconcilerDispatchCtx>,
        plan_id: Option<String>,
        feature_gate: Arc<spur_license::FeatureGate>,
    ) -> Self {
        Self::new_with_pm_like(
            config,
            pm as Arc<dyn crate::plan::PmLike>,
            fast_forward,
            dispatch.map(|dispatch| Arc::new(dispatch) as Arc<dyn ReconcilerDispatch>),
            plan_id,
            feature_gate,
        )
    }

    pub fn new_with_pm_like(
        config: ReconcilerConfig,
        pm: Arc<dyn crate::plan::PmLike>,
        fast_forward: Arc<Notify>,
        dispatch: Option<Arc<dyn ReconcilerDispatch>>,
        plan_id: Option<String>,
        feature_gate: Arc<spur_license::FeatureGate>,
    ) -> Self {
        Self {
            config,
            pm,
            fast_forward,
            dispatch,
            plan_id,
            auto_merge_approved_plans: false,
            automation: None,
            journal_wake: None,
            feature_gate,
            outcomes: Arc::new(tokio::sync::Mutex::new(OutcomeStore::default())),
            clock: Arc::new(SystemClock),
            hygiene: tokio::sync::Mutex::new(hygiene::HygieneState::default()),
        }
    }

    pub fn set_outcomes(&mut self, outcomes: Arc<tokio::sync::Mutex<OutcomeStore>>) {
        self.outcomes = outcomes;
    }

    pub fn outcomes(&self) -> Arc<tokio::sync::Mutex<OutcomeStore>> {
        Arc::clone(&self.outcomes)
    }

    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = clock;
    }

    pub fn set_journal_wake(&mut self, notify: Arc<Notify>) {
        self.journal_wake = Some(notify);
    }

    pub fn set_auto_merge_approved_plans(&mut self, enabled: bool) {
        self.auto_merge_approved_plans = enabled;
    }

    pub fn set_automation(&mut self, automation: Arc<dyn ReconcilerAutomation>) {
        self.automation = Some(automation);
    }

    fn now(&self) -> SystemTime {
        self.clock.now()
    }

    async fn mark_tick(&self) {
        let now = self.now();
        self.outcomes.lock().await.mark_tick(now);
    }

    async fn record_tick_plans_enumerated(&self, count: usize) {
        self.outcomes
            .lock()
            .await
            .record_tick_plans_enumerated(count);
    }

    async fn record_outcome(&self, plan_id: Option<&str>, outcome: DispatchOutcome) {
        self.outcomes.lock().await.record_outcome(plan_id, outcome);
    }

    async fn record_no_ready(&self, plan_id: Option<&str>) {
        let now = self.now();
        let outcome = DispatchOutcome::NoReadyTasks {
            plan_id: plan_id.unwrap_or("*").to_string(),
            reason: NoReadyReason::NoMatchingRows,
            timestamp: now,
        };
        self.record_outcome(plan_id, outcome).await;
    }

    async fn record_no_dispatch_context(&self, ready_count: usize) {
        let now = self.now();
        self.outcomes.lock().await.record_no_dispatch_context(
            self.plan_id.as_deref(),
            ready_count,
            now,
        );
    }

    async fn record_dispatched(
        &self,
        plan_id: &str,
        task_id: &str,
        agent: &str,
        delegation_id: &str,
        agent_fallback: bool,
    ) {
        let now = self.now();
        self.outcomes.lock().await.record_dispatched(
            plan_id,
            task_id,
            agent,
            delegation_id,
            agent_fallback,
            now,
        );
    }

    async fn record_skipped(&self, plan_id: Option<&str>, task_id: &str, reason: SkipReason) {
        let now = self.now();
        let reason_for_log = reason.clone();
        let decision = self
            .outcomes
            .lock()
            .await
            .record_skipped(plan_id, task_id, reason, now);
        self.log_skip_decision(plan_id, task_id, &reason_for_log, decision);
    }

    fn log_skip_decision(
        &self,
        plan_id: Option<&str>,
        task_id: &str,
        reason: &SkipReason,
        decision: OutcomeLogDecision,
    ) {
        if decision.state_changed {
            tracing::info!(
                plan_id = plan_id.unwrap_or(""),
                %task_id,
                ?reason,
                "reconciler skip reason changed"
            );
        }
        if let Some(StuckTask { stuck_since, .. }) = decision.stuck_warn {
            tracing::warn!(
                plan_id = plan_id.unwrap_or(""),
                %task_id,
                ?reason,
                ?stuck_since,
                "reconciler task stuck on skip reason"
            );
        }
    }

    async fn project_plan_from_beads(
        &self,
        plan_id: &str,
    ) -> anyhow::Result<crate::plan::PlanState> {
        let projected = crate::plan::projector::project_plan_from_beads(
            self.pm.as_ref(),
            plan_id,
            self.feature_gate.as_ref(),
        )
        .await?;
        prune_projected_terminal_task_outcomes(&self.outcomes, plan_id, &projected.tasks).await;
        Ok(projected)
    }

    async fn epic_labels_for_plan(
        &self,
        plan_id: &str,
        epic_id: Option<&str>,
        cache: &mut HashMap<String, Vec<String>>,
    ) -> anyhow::Result<Vec<String>> {
        if let Some(labels) = cache.get(plan_id) {
            return Ok(labels.clone());
        }

        let labels = if let Some(epic_id) = epic_id {
            self.pm.get_issue(epic_id).await?.labels
        } else {
            self.pm
                .list_issues(spur_pm::IssueFilter {
                    labels: vec![
                        crate::plan::labels::plan_id(plan_id),
                        crate::plan::labels::PLAN_COMPLETE.to_string(),
                    ],
                    issue_type: Some("epic".into()),
                    limit: Some(1),
                    ..Default::default()
                })
                .await?
                .into_iter()
                .next()
                .map(|summary| summary.labels)
                .unwrap_or_default()
        };

        cache.insert(plan_id.to_string(), labels.clone());
        Ok(labels)
    }

    async fn active_plan_handle(
        &self,
        plan_id: &str,
    ) -> Option<Arc<tokio::sync::Mutex<crate::plan::PlanState>>> {
        match self.project_plan_from_beads(plan_id).await {
            Ok(projected) => Some(Arc::new(tokio::sync::Mutex::new(projected))),
            Err(error) => {
                tracing::warn!(
                    %plan_id,
                    "predispatch preview: failed to project active plan handle: {error}"
                );
                None
            }
        }
    }

    async fn emit_snapshot_for_plan(&self, plan_id: &str) {
        let Some(sink) = self
            .dispatch
            .as_ref()
            .and_then(|dispatch| dispatch.event_sink())
        else {
            return;
        };

        match self.project_plan_from_beads(plan_id).await {
            Ok(projected) => {
                crate::plan::snapshot::emit_plan_snapshot(Some(sink.as_ref()), &projected)
            }
            Err(error) => tracing::warn!(%plan_id, "failed to project plan snapshot: {error}"),
        }
    }

    pub async fn run(self, cancel: tokio::sync::oneshot::Receiver<()>) {
        let mut interval = self.config.base_interval;
        let journal_wake = self.journal_wake.clone();
        tokio::pin!(cancel);
        loop {
            let journal_fut = async {
                if let Some(ref n) = journal_wake {
                    n.notified().await;
                } else {
                    std::future::pending().await
                }
            };
            tokio::pin!(journal_fut);
            tokio::select! {
                _ = &mut cancel => {
                    tracing::info!("reconciler received cancel");
                    break;
                }
                _ = self.fast_forward.notified() => {
                    tracing::debug!("reconciler fast-forward triggered");
                    interval = self.config.base_interval;
                }
                _ = &mut journal_fut => {
                    tracing::debug!("reconciler journal append triggered");
                    interval = self.config.base_interval;
                }
                _ = self.clock.sleep(interval) => {}
            }
            // Race tick_once against cancel so shutdown cannot hang behind
            // stuck PM I/O (bv.triage / br ready). Partial persisted writes are
            // level-triggered and compensated on the next tick / restart.
            let did_work = tokio::select! {
                biased;
                _ = &mut cancel => {
                    tracing::info!("reconciler received cancel during tick");
                    break;
                }
                result = self.tick_once() => {
                    match result {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::warn!("reconciler tick failed: {e}");
                            false
                        }
                    }
                }
            };
            if did_work {
                interval = self.config.base_interval;
            } else {
                let scaled = interval.saturating_mul(self.config.backoff_factor);
                interval = std::cmp::min(scaled, self.config.idle_ceiling);
            }
        }
    }

    pub async fn tick_once(&self) -> anyhow::Result<bool> {
        self.mark_tick().await;
        let mut did_work = self.reconcile_plan_owner_leases().await?;
        did_work |= self.reconcile_system_l3_reviews().await?;
        did_work |= self.reconcile_terminal_epics().await?;

        let Some(dispatch) = &self.dispatch else {
            let ready_ids = self.observe_ready().await?;
            for id in &ready_ids {
                tracing::debug!(%id, "reconciler observed ready task");
            }
            if !ready_ids.is_empty() {
                self.record_no_dispatch_context(ready_ids.len()).await;
            }
            did_work |= self.run_index_hygiene_sweep().await?;
            return Ok(did_work || !ready_ids.is_empty());
        };

        did_work |= self
            .sweep_expired_dispatch_leases(dispatch.as_ref())
            .await?;
        did_work |= self.run_loop_scheduler_sweep().await?;

        let ready = self.observe_ready_summaries().await?;
        let mut epic_labels_cache = HashMap::new();

        for hydrated in ready {
            let summary = hydrated.summary;
            let Some(plan_id) = summary
                .labels
                .iter()
                .find_map(|label| crate::plan::labels::parse_plan_id(label))
            else {
                self.record_skipped(None, &summary.id, SkipReason::MissingPlanId)
                    .await;
                continue;
            };

            let projected = match projected_plan_for_ready(
                hydrated.plan_state,
                self.project_plan_from_beads(plan_id),
            )
            .await
            {
                Ok(projected) => projected,
                Err(error) => {
                    tracing::warn!(
                        issue_id = %summary.id,
                        %plan_id,
                        "reconciler skipping ready summary after plan projection failed: {error}"
                    );
                    self.record_skipped(
                        Some(plan_id),
                        &summary.id,
                        SkipReason::ProjectorFailed {
                            error: error.to_string(),
                        },
                    )
                    .await;
                    continue;
                }
            };
            let Some(task) = projected
                .tasks
                .iter()
                .find(|task| task.spec.issue_id.as_deref() == Some(summary.id.as_str()))
            else {
                self.record_skipped(
                    Some(plan_id),
                    &summary.id,
                    SkipReason::TaskMissingFromProjection,
                )
                .await;
                continue;
            };
            if !matches!(task.status, crate::plan::PlanTaskStatus::Ready) {
                let blocked_by = unresolved_blocker_issue_ids(&projected, task);
                self.record_skipped(
                    Some(plan_id),
                    &task.spec.task_id,
                    SkipReason::TaskStatusNotReady { blocked_by },
                )
                .await;
                continue;
            }
            let epic_labels = match self
                .epic_labels_for_plan(
                    plan_id,
                    projected.epic_id.as_deref(),
                    &mut epic_labels_cache,
                )
                .await
            {
                Ok(labels) => labels,
                Err(error) => {
                    tracing::warn!(
                        issue_id = %summary.id,
                        %plan_id,
                        task_id = %task.spec.task_id,
                        "reconciler skipping ready task after epic label hydration failed: {error}"
                    );
                    self.record_skipped(
                        Some(plan_id),
                        &task.spec.task_id,
                        SkipReason::HydrationGetIssueFailed {
                            error: error.to_string(),
                        },
                    )
                    .await;
                    continue;
                }
            };
            if let Some(state) = report_only_dispatch_state(
                &epic_labels,
                &summary.labels,
                projected.epic_id.as_deref(),
            ) {
                if let Some(reason) = state.skip_reason() {
                    self.record_skipped(Some(plan_id), &task.spec.task_id, reason)
                        .await;
                }
                continue;
            }

            let delegation_id = crate::plan::labels::mint_delegation_id();
            let task_attempt = task.attempt;
            let agent_fallback = !summary
                .labels
                .iter()
                .any(|label| crate::plan::labels::parse_agent(label).is_some());
            let base_spec = match plan_dispatch_base_spec(
                &projected,
                &task.spec.task_id,
                &self.config.repo_root,
            )
            .await
            {
                Ok(base_spec) => base_spec,
                Err(error) => {
                    tracing::warn!(
                        issue_id = %summary.id,
                        %plan_id,
                        task_id = %task.spec.task_id,
                        "reconciler skipping ready task after base spec build failed: {error}"
                    );
                    self.record_skipped(
                        Some(plan_id),
                        &task.spec.task_id,
                        SkipReason::BaseSpecBuildFailed {
                            error: error.to_string(),
                        },
                    )
                    .await;
                    continue;
                }
            };
            let has_overlays = matches!(
                &base_spec,
                crate::BaseSpec::WithOverlay { overlays, .. } if !overlays.is_empty()
            );
            let preview_outcome = if has_overlays {
                match &self.config.predispatch_preview {
                    PreviewStrategy::AlwaysClean => None,
                    PreviewStrategy::AlwaysConflict { dep_task_id, files } => {
                        Some(crate::tool_schemas::PreviewConflict {
                            dep_task_id: dep_task_id.clone(),
                            files: files.clone(),
                        })
                    }
                    PreviewStrategy::Real => match self.active_plan_handle(plan_id).await {
                        Some(plan_arc) => {
                            match crate::plan::preview::preview_overlay(
                                &plan_arc,
                                plan_id,
                                &task.spec.task_id,
                                &self.config.repo_root,
                            )
                            .await
                            {
                                Ok(output) => output.conflict,
                                Err(error) => {
                                    tracing::warn!(
                                        %plan_id,
                                        task_id = %task.spec.task_id,
                                        "predispatch preview: helper errored, falling through to live dispatch: {error}"
                                    );
                                    None
                                }
                            }
                        }
                        None => {
                            tracing::warn!(
                                %plan_id,
                                task_id = %task.spec.task_id,
                                "predispatch preview: no active plan handle; skipping check"
                            );
                            None
                        }
                    },
                }
            } else {
                None
            };
            if let Some(conflict) = preview_outcome {
                tracing::info!(
                    %plan_id,
                    task_id = %task.spec.task_id,
                    dep_task_id = %conflict.dep_task_id,
                    files = ?conflict.files,
                    "predispatch preview: overlay conflict predicted; blocking without worker spawn"
                );
                if let Err(error) = self
                    .transition_to_blocked_on_setup_conflict(
                        plan_id,
                        &task.spec.task_id,
                        &conflict.dep_task_id,
                        &conflict.files,
                    )
                    .await
                {
                    tracing::warn!(
                        %plan_id,
                        task_id = %task.spec.task_id,
                        "predispatch preview: failed to persist setup-conflict transition: {error}"
                    );
                    self.record_skipped(
                        Some(plan_id),
                        &task.spec.task_id,
                        SkipReason::PersistError {
                            msg: error.to_string(),
                        },
                    )
                    .await;
                    continue;
                }

                // bd-88r — compile verified git topology and push a brain
                // continuation so the brain does not hallucinate parentage.
                if let Some(dispatch) = self.dispatch.as_ref() {
                    emit_setup_conflict_continuation(SetupConflictContinuation {
                        plan: &projected,
                        repo_root: &self.config.repo_root,
                        task_id: &task.spec.task_id,
                        delegation_id: &delegation_id,
                        attempt: task_attempt,
                        dep_task_id: &conflict.dep_task_id,
                        files: &conflict.files,
                        summary: "Predispatch overlay preview predicted a setup conflict.",
                        event_sink: dispatch.event_sink().map(|sink| sink.as_ref()),
                        continuation_ctx: dispatch.continuation_ctx().as_ref(),
                    })
                    .await;
                }

                self.record_skipped(
                    Some(plan_id),
                    &task.spec.task_id,
                    SkipReason::PredispatchOverlayConflict {
                        dep_task_id: conflict.dep_task_id,
                        files: conflict.files,
                    },
                )
                .await;
                continue;
            }
            if let Err(error) = crate::plan::persist_dispatch_intent(
                self.pm.as_ref(),
                &summary.id,
                self.feature_gate.as_ref(),
                plan_id,
                &delegation_id,
                &task.spec.agent,
                task_attempt,
                self.config.dispatch_lease_duration,
            )
            .await
            {
                tracing::warn!(
                    issue_id = %summary.id,
                    %plan_id,
                    task_id = %task.spec.task_id,
                    "reconciler skipping ready task after persist_dispatch_intent failed: {error}"
                );
                self.record_skipped(
                    Some(plan_id),
                    &task.spec.task_id,
                    SkipReason::PersistDispatchIntentFailed {
                        error: error.to_string(),
                    },
                )
                .await;
                continue;
            }

            let (respond_to, rx) = tokio::sync::oneshot::channel();
            let (dispatched_base_oid_tx, dispatched_base_oid_rx) =
                tokio::sync::watch::channel(None);
            let task_text = crate::plan::build_dispatch_task_text(task);
            let request = crate::DelegationRequest {
                id: delegation_id.clone().into(),
                agent: task.spec.agent.clone(),
                profile: task.spec.profile.clone(),
                skills: task.spec.skills.clone(),
                model: task.spec.model.clone(),
                effort: task.spec.effort.clone(),
                config_overrides: task.spec.config_overrides.clone(),
                task: task_text,
                context_files: task.spec.context_files.clone(),
                prior_branch_for_reuse: prior_branch_for_reuse(task),
                respond_to,
                brain_session_id: dispatch.brain_session_id().clone(),
                delegation_plan: None,
                issue_id: task.spec.issue_id.clone(),
                base: Some(base_spec),
                dispatched_base_oid_tx: Some(dispatched_base_oid_tx),
                attempt_tracker: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(
                    task_attempt,
                )),
                enable_worker_mcp: None,
            };

            // INV-S3 audit: dispatch intent is durable in beads before this
            // in-process request becomes observable by the orchestrator.
            if let Err(error) = dispatch.send_delegation(request).await {
                self.record_skipped(
                    Some(plan_id),
                    &task.spec.task_id,
                    SkipReason::DispatchSendFailed {
                        msg: error.to_string(),
                    },
                )
                .await;
                if let Err(e) = crate::plan::clear_dispatch_intent(
                    self.pm.as_ref(),
                    &summary.id,
                    &delegation_id,
                )
                .await
                {
                    tracing::warn!(
                        target: "spur.dispatch_intent_cleanup",
                        %plan_id,
                        task_id = %task.spec.task_id,
                        %delegation_id,
                        "dispatch cleanup failed: {e}"
                    );
                }
                let mut update = crate::plan::dispatch_send_failure_update(&delegation_id, &[]);
                update.remove_labels.clear();
                if let Err(e) = self.pm.update_issue(&summary.id, update).await {
                    tracing::warn!(
                        target: "spur.dispatch_intent_cleanup",
                        %plan_id,
                        task_id = %task.spec.task_id,
                        %delegation_id,
                        "dispatch cleanup failed: {e}"
                    );
                }
                tracing::warn!(
                    issue_id = %summary.id,
                    %delegation_id,
                    "reconciler send failed: {error}"
                );
                continue;
            }

            self.record_dispatched(
                plan_id,
                &task.spec.task_id,
                &task.spec.agent,
                &delegation_id,
                agent_fallback,
            )
            .await;
            spur_telemetry::emit!(spur_telemetry::tier2_events::WorkerDispatched {
                worker_model: crate::telemetry::model_name_from_config_value(
                    task.spec.model.as_deref(),
                    "worker",
                ),
                skill_used: spur_telemetry::tier2_events::SkillName::Other,
                attempt_num: task_attempt,
            });
            self.emit_snapshot_for_plan(plan_id).await;

            let pm = Arc::clone(&self.pm);
            let plan_id = plan_id.to_string();
            let task_id = task.spec.task_id.clone();
            let issue_id = summary.id.clone();
            let delegation_id_for_completion = delegation_id.clone();
            let repo_root = self.config.repo_root.clone();
            let fast_forward = Arc::clone(&self.fast_forward);
            let event_sink = dispatch.event_sink().cloned();
            let brain_session_id = dispatch.brain_session_id().clone();
            let materializer = Arc::clone(dispatch.materializer());
            let continuation_ctx = Arc::clone(dispatch.continuation_ctx());
            let feature_gate = Arc::clone(&self.feature_gate);
            let outcomes = Arc::clone(&self.outcomes);
            let clock = Arc::clone(&self.clock);
            let completion_log_context = CompletionCollectorLogContext {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                delegation_id: delegation_id_for_completion.clone(),
                brain_session_id: brain_session_id.to_string(),
                attempt: task_attempt,
            };
            spawn_completion_collector(dispatch.as_ref(), completion_log_context, async move {
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::warn!(
                            %plan_id,
                            %task_id,
                            %issue_id,
                            %delegation_id_for_completion,
                            "reconciler completion receiver dropped before result persisted"
                        );
                        let error = "orchestrator disconnected".to_string();
                        spur_acp::DelegationResult {
                            resolved_config: None,
                            status: spur_acp::DelegationStatus::Failed {
                                error: error.clone(),
                            },
                            diff: None,
                            diff_summary: None,
                            summary: Some(error),
                            estimated_cost_usd: 0.0,
                            worker_branch: None,
                            artifact: None,
                        }
                    }
                };

                if let Some((source_task_id, files)) =
                    conflict::setup_overlay_conflict(&result.status)
                {
                    if let Err(error) = conflict::persist_setup_overlay_conflict(
                        pm.as_ref(),
                        &issue_id,
                        feature_gate.as_ref(),
                        &plan_id,
                        &delegation_id_for_completion,
                        source_task_id,
                        files,
                    )
                    .await
                    {
                        tracing::warn!(
                            %plan_id,
                            %task_id,
                            %issue_id,
                            %delegation_id_for_completion,
                            "reconciler setup conflict persistence failed: {error}"
                        );
                    } else {
                        fast_forward.notify_one();
                    }

                    match crate::plan::projector::project_plan_from_beads(
                        pm.as_ref(),
                        &plan_id,
                        feature_gate.as_ref(),
                    )
                    .await
                    {
                        Ok(projected) => {
                            if let Some(sink) = event_sink.as_deref() {
                                crate::plan::snapshot::emit_plan_snapshot(Some(sink), &projected);
                            }
                            emit_setup_conflict_continuation(SetupConflictContinuation {
                                plan: &projected,
                                repo_root: &repo_root,
                                task_id: &task_id,
                                delegation_id: &delegation_id_for_completion,
                                attempt: task_attempt,
                                dep_task_id: source_task_id,
                                files,
                                summary: "Worker setup failed with an overlay conflict.",
                                event_sink: event_sink.as_deref(),
                                continuation_ctx: continuation_ctx.as_ref(),
                            })
                            .await;
                        }
                        Err(error) => tracing::warn!(
                            %plan_id,
                            %task_id,
                            "failed to project plan snapshot after setup conflict: {error}"
                        ),
                    }
                    return;
                }

                // INV-S3 audit: the worker result and base-OID watch value are
                // consumed only by this completion writer; events,
                // continuations, and projected state are emitted after the
                // completion audit/update below succeeds.
                let dispatched_base_oid = dispatched_base_oid_rx.borrow().clone();

                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    %plan_id,
                    %task_id,
                    delegation_id = %delegation_id_for_completion,
                    brain_session_id = %brain_session_id,
                    attempt = task_attempt,
                    "stage_entered_persist"
                );
                let deferred = match crate::plan::persist_worker_completion_and_notify(
                    pm.as_ref(),
                    &issue_id,
                    feature_gate.as_ref(),
                    &plan_id,
                    &delegation_id_for_completion,
                    &Some(Arc::clone(&fast_forward)),
                    &result,
                    &brain_session_id,
                    task_attempt,
                    &materializer,
                    dispatched_base_oid,
                    Some(repo_root.clone()),
                    Some(&task_id),
                )
                .await
                {
                    Ok(payload) => payload,
                    Err(error) => {
                        tracing::warn!(
                            %plan_id,
                            %task_id,
                            %issue_id,
                            %delegation_id_for_completion,
                            "reconciler completion persistence failed: {error}"
                        );
                        None
                    }
                };
                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    %plan_id,
                    %task_id,
                    delegation_id = %delegation_id_for_completion,
                    brain_session_id = %brain_session_id,
                    attempt = task_attempt,
                    deferred_is_some = deferred.is_some(),
                    "stage_completed_persist"
                );

                tracing::info!(
                    target: "spur.reconciler.completion_collector",
                    %plan_id,
                    %task_id,
                    delegation_id = %delegation_id_for_completion,
                    brain_session_id = %brain_session_id,
                    attempt = task_attempt,
                    "stage_entered_project"
                );
                project_completion_snapshot_and_deliver(
                    clock.as_ref(),
                    crate::plan::projector::project_plan_from_beads(
                        pm.as_ref(),
                        &plan_id,
                        feature_gate.as_ref(),
                    ),
                    &outcomes,
                    event_sink.as_deref(),
                    continuation_ctx.as_ref(),
                    deferred,
                    CompletionProjectionLogContext {
                        plan_id: &plan_id,
                        task_id: &task_id,
                        delegation_id: &delegation_id_for_completion,
                        brain_session_id: &brain_session_id,
                        attempt: task_attempt,
                    },
                )
                .await;
            });

            did_work = true;
        }

        did_work |= self.run_index_hygiene_sweep().await?;

        Ok(did_work)
    }

    async fn index_hygiene_sweep(
        &self,
        adv: &dyn spur_pm::BeadsAdvanced,
        issue: &spur_pm::IssueSummary,
        audits: &[crate::plan::audit_sentinel::AuditSentinelKind],
        parent_ids: Option<&[String]>,
    ) -> anyhow::Result<bool> {
        if audits.is_empty() {
            return self.reconcile_label_only_dispatch(issue).await;
        }
        let terminal = crate::plan::projector::terminal_status_from_audits(audits);
        let expected_delegation = if terminal.is_some() {
            None
        } else {
            crate::plan::projector::current_delegation_from_audits(audits)
        };
        let expected_ready_for_review =
            terminal.is_none() && crate::plan::projector::awaiting_review_from_audits(audits);
        let expected_review_rejected = matches!(
            terminal,
            Some(crate::plan::projector::TerminalAuditKind::Rejection)
        );
        let expected_plan_id_value = match expected_plan_id_from_audits(audits) {
            Some(plan_id) => Some(plan_id),
            None => match parent_ids {
                Some(parents) => {
                    self.expected_plan_id_from_parents(adv, issue, parents)
                        .await?
                }
                None => self.expected_plan_id_from_parent_epic(adv, issue).await?,
            },
        };
        let expected_plan_id =
            expected_plan_id_value.map(|plan_id| crate::plan::labels::plan_id(&plan_id));
        let expected_plan_task_id = expected_plan_task_id_from_audits(audits)
            .map(|task_id| crate::plan::labels::plan_task_id(&task_id));
        let expected_agent =
            expected_agent_from_audits(audits).map(|agent| crate::plan::labels::agent(&agent));
        let expected_plan_complete =
            expected_plan_complete_from_audits(&issue.id, audits).unwrap_or(false);
        let expected_plan_pending =
            expected_plan_pending_from_audits(&issue.id, audits).unwrap_or(false);

        let mut add_labels = Vec::new();
        let mut remove_labels = Vec::new();
        let mut drift_events = Vec::new();

        // Keep the raw existing label string (matching plan_id/plan_task_id/agent
        // collection below). If we canonicalized to `spur:delegation-id:X` here,
        // the backend would be asked to remove the canonical form while the issue
        // actually carries the legacy `delegation-id:X` form — leaking it forever.
        // `reconcile_singleton_label` does the legacy→canonical conversion via
        // its mismatch path: stale legacy is pushed to remove_labels with its
        // raw value, and the canonical expected is pushed to add_labels.
        let existing_delegations = issue
            .labels
            .iter()
            .filter(|label| delegation_label_value(label).is_some())
            .cloned()
            .collect::<Vec<_>>();
        let expected_delegation_label = expected_delegation
            .as_deref()
            .map(crate::plan::labels::delegation_id);
        reconcile_singleton_label(
            "delegation_id",
            expected_delegation_label,
            existing_delegations,
            &mut add_labels,
            &mut remove_labels,
            &mut drift_events,
        );
        reconcile_presence_label(
            "ready_for_review",
            crate::plan::labels::READY_FOR_REVIEW,
            is_ready_for_review_label,
            expected_ready_for_review,
            &issue.labels,
            &mut LabelReconcileBuffers {
                add_labels: &mut add_labels,
                remove_labels: &mut remove_labels,
                drift_events: &mut drift_events,
            },
        );
        reconcile_presence_label(
            "review_rejected",
            crate::plan::labels::REVIEW_REJECTED,
            is_review_rejected_label,
            expected_review_rejected,
            &issue.labels,
            &mut LabelReconcileBuffers {
                add_labels: &mut add_labels,
                remove_labels: &mut remove_labels,
                drift_events: &mut drift_events,
            },
        );
        let existing_plan_ids = issue
            .labels
            .iter()
            .filter(|label| crate::plan::labels::parse_plan_id(label).is_some())
            .cloned()
            .collect::<Vec<_>>();
        reconcile_singleton_label(
            "plan_id",
            expected_plan_id,
            existing_plan_ids,
            &mut add_labels,
            &mut remove_labels,
            &mut drift_events,
        );
        let existing_plan_task_ids = issue
            .labels
            .iter()
            .filter(|label| crate::plan::labels::parse_plan_task_id(label).is_some())
            .cloned()
            .collect::<Vec<_>>();
        reconcile_singleton_label(
            "plan_task_id",
            expected_plan_task_id,
            existing_plan_task_ids,
            &mut add_labels,
            &mut remove_labels,
            &mut drift_events,
        );
        let existing_agents = issue
            .labels
            .iter()
            .filter(|label| crate::plan::labels::parse_agent(label).is_some())
            .cloned()
            .collect::<Vec<_>>();
        reconcile_singleton_label(
            "agent",
            expected_agent,
            existing_agents,
            &mut add_labels,
            &mut remove_labels,
            &mut drift_events,
        );
        reconcile_presence_label(
            "plan_complete",
            crate::plan::labels::PLAN_COMPLETE,
            |label| label == crate::plan::labels::PLAN_COMPLETE,
            expected_plan_complete,
            &issue.labels,
            &mut LabelReconcileBuffers {
                add_labels: &mut add_labels,
                remove_labels: &mut remove_labels,
                drift_events: &mut drift_events,
            },
        );
        reconcile_presence_label(
            "plan_pending",
            crate::plan::labels::PLAN_PENDING,
            |label| label == crate::plan::labels::PLAN_PENDING,
            expected_plan_pending,
            &issue.labels,
            &mut LabelReconcileBuffers {
                add_labels: &mut add_labels,
                remove_labels: &mut remove_labels,
                drift_events: &mut drift_events,
            },
        );
        // Intentionally exempt `spur:superseded-by:*` + `spur:mutation-id:*` from
        // index-hygiene repair. These families are irreducible label-only fields:
        // `partial_compare_status` accepts legacy Superseded data that shadow
        // projection cannot reconstruct from audits (`projector.rs:412-439`).

        add_labels.sort();
        add_labels.dedup();
        remove_labels.sort();
        remove_labels.dedup();

        if add_labels.is_empty() && remove_labels.is_empty() {
            return Ok(false);
        }

        for event in &drift_events {
            emit_label_index_drift(event);
        }

        // TOCTOU note: comments/audits may change between `list_comments` and this
        // write. The sweep is idempotent and converges on the next tick.
        self.pm
            .update_issue(
                &issue.id,
                spur_pm::IssueUpdate {
                    add_labels,
                    remove_labels,
                    ..Default::default()
                },
            )
            .await?;
        Ok(true)
    }

    async fn reconcile_label_only_dispatch(
        &self,
        issue: &spur_pm::IssueSummary,
    ) -> anyhow::Result<bool> {
        let remove_labels = issue
            .labels
            .iter()
            .filter(|label| delegation_label_value(label).is_some())
            .cloned()
            .chain(
                issue
                    .labels
                    .iter()
                    .filter(|label| crate::plan::labels::parse_lease_expires_at(label).is_some())
                    .cloned(),
            )
            .collect::<Vec<_>>();
        if remove_labels.is_empty() {
            return Ok(false);
        }

        let detail = self.pm.get_issue(&issue.id).await?;
        let now: chrono::DateTime<chrono::Utc> = self.now().into();
        let age = now.signed_duration_since(detail.updated_at);
        if age
            .to_std()
            .map(|age| age < self.config.label_only_dispatch_grace)
            .unwrap_or(true)
        {
            return Ok(false);
        }

        self.pm
            .update_issue(
                &issue.id,
                spur_pm::IssueUpdate {
                    remove_labels,
                    ..Default::default()
                },
            )
            .await?;
        Ok(true)
    }

    async fn expected_plan_id_from_parent_epic(
        &self,
        adv: &dyn spur_pm::BeadsAdvanced,
        issue: &spur_pm::IssueSummary,
    ) -> anyhow::Result<Option<String>> {
        // `Issue::blocked_by` is the active scheduling view and intentionally
        // excludes parent-child edges. Recover ownership from the structural
        // graph instead so an unrelated blocker cannot donate its plan ID.
        let graph = self.pm.issue_subgraph_json(&issue.id).await?;
        let adjacency = graph.adjacency.ok_or_else(|| {
            anyhow::anyhow!(
                "issue graph for '{}' did not return JSON adjacency",
                issue.id
            )
        })?;
        let parents = adjacency
            .edges
            .unwrap_or_default()
            .into_iter()
            .filter(|edge| edge.to == issue.id && edge.edge_type.as_deref() == Some("parent-child"))
            .map(|edge| edge.from)
            .collect::<Vec<_>>();
        self.expected_plan_id_from_parents(adv, issue, &parents)
            .await
    }
}

#[cfg(test)]
mod tests;
