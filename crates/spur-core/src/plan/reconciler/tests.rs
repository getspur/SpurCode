use super::*;
use crate::plan::PmLike;
use proptest::prelude::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Notify;
use tracing::field::{Field, Visit};

fn pro_feature_gate() -> Arc<spur_license::FeatureGate> {
    let gate = Arc::new(spur_license::FeatureGate::new(
        spur_license::policy::PolicyResolver::embedded(),
    ));
    let features =
        std::collections::BTreeSet::from([spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED
            .as_str()
            .to_string()]);
    gate.update_state(&spur_license::LicenseState::active_validated(
        spur_license::Plan::Pro,
        features,
    ));
    gate
}

fn test_completion_dispatch(
    task_tracker: crate::server::AbortableTaskTracker,
    brain_session_id: impl Into<String>,
) -> ReconcilerDispatchCtx {
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel(1);
    ReconcilerDispatchCtx {
        delegation_tx,
        task_tracker,
        brain_session_id: spur_acp::BrainSessionId::new(spur_acp::SessionId(
            brain_session_id.into(),
        )),
        event_sink: None,
        materializer: Arc::new(crate::outcome_materializer::OutcomeMaterializer::new(
            Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
        )),
        continuation_ctx: Arc::new(crate::plan::continuation::DetachedContinuationCtx {
            on_complete: Arc::new(|_, _| Box::pin(async {})),
        }),
    }
}

struct FixedClock {
    now: SystemTime,
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.now
    }
}

fn projected_test_state(plan_id: &str) -> crate::plan::PlanState {
    crate::plan::PlanState {
        plan_id: plan_id.to_string(),
        tasks: Vec::new(),
        brain_session_id: spur_acp::BrainSessionId::new(spur_acp::SessionId("test-brain".into())),
        base_snapshot_branch: None,
        base_snapshot_oid: None,
        merge_state: crate::plan::PlanMergeState::NotStarted,
        epic_id: None,
    }
}

#[derive(Clone, Default)]
struct CapturedCompletionCollectorErrors {
    events: Arc<std::sync::Mutex<Vec<CapturedCompletionCollectorEvent>>>,
}

#[derive(Default)]
struct CapturedCompletionCollectorEvent {
    target: String,
    fields: String,
}

impl CapturedCompletionCollectorErrors {
    fn contains_event_with(&self, needles: &[&str]) -> bool {
        self.events.lock().unwrap().iter().any(|event| {
            event.target == "spur.reconciler.completion_collector"
                && needles.iter().all(|needle| event.fields.contains(needle))
        })
    }
}

impl tracing::Subscriber for CapturedCompletionCollectorErrors {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::INFO
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = CompletionCollectorVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .unwrap()
            .push(CapturedCompletionCollectorEvent {
                target: event.metadata().target().to_string(),
                fields: visitor.0,
            });
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[derive(Default)]
struct CompletionCollectorVisitor(String);

impl Visit for CompletionCollectorVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(&format!("{value:?}"));
        self.0.push(' ');
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(value);
        self.0.push(' ');
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(&value.to_string());
        self.0.push(' ');
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(&value.to_string());
        self.0.push(' ');
    }
}

#[tokio::test]
async fn completion_collector_logs_panic_with_structured_context() {
    let captured = CapturedCompletionCollectorErrors::default();
    let tracker = crate::server::AbortableTaskTracker::new();
    let dispatch = test_completion_dispatch(tracker.clone(), "brain-panic");
    let context = CompletionCollectorLogContext {
        plan_id: "plan-panic".into(),
        task_id: "task-panic".into(),
        delegation_id: "del-panic".into(),
        brain_session_id: "brain-panic".into(),
        attempt: 7,
    };

    let guard = tracing::subscriber::set_default(captured.clone());
    spawn_completion_collector(&dispatch, context, async {
        panic!("H1 simulated completion collector panic");
    });
    tracker.close();
    tracker.wait().await;
    drop(guard);

    assert!(
        captured.contains_event_with(&[
            "plan_id=plan-panic",
            "task_id=task-panic",
            "delegation_id=del-panic",
            "brain_session_id=brain-panic",
            "attempt=7",
            "H1 simulated completion collector panic",
            "backtrace=",
        ]),
        "expected structured panic event, got {:?}",
        captured
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|event| format!("{} {}", event.target, event.fields))
            .collect::<Vec<_>>()
    );
}

#[derive(Default)]
struct RecordingEventSink {
    events: std::sync::Mutex<Vec<spur_acp::SpurEventBody>>,
}

impl spur_mcp::events::McpEventSink for RecordingEventSink {
    fn emit(&self, event: spur_acp::SpurEventBody) {
        self.events.lock().expect("events lock").push(event);
    }
}

#[derive(Default)]
struct CompletionTimeoutAdvanced {
    comments: std::sync::Mutex<Vec<spur_pm::Comment>>,
}

#[async_trait::async_trait]
impl spur_pm::BeadsAdvanced for CompletionTimeoutAdvanced {
    async fn list_ready(
        &self,
        _filter: spur_pm::ReadyFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        Ok(Vec::new())
    }

    async fn list_comments(&self, _issue_id: &str) -> anyhow::Result<Vec<spur_pm::Comment>> {
        Ok(self.comments.lock().expect("comments lock").clone())
    }

    async fn add_comment(&self, _issue_id: &str, body: &str) -> anyhow::Result<String> {
        let mut comments = self.comments.lock().expect("comments lock");
        let id = format!("c{}", comments.len() + 1);
        comments.push(spur_pm::Comment {
            id: id.clone(),
            body: body.to_string(),
            actor: "spur-test".to_string(),
            created_at: chrono::Utc::now(),
        });
        Ok(id)
    }

    async fn remove_dependency(&self, _issue_id: &str, _depends_on_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn dep_cycles(&self) -> anyhow::Result<Vec<spur_pm::DependencyCycle>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct CompletionTimeoutPm {
    advanced: CompletionTimeoutAdvanced,
    labels: std::sync::Mutex<Vec<String>>,
    updates: std::sync::Mutex<Vec<spur_pm::IssueUpdate>>,
}

#[async_trait::async_trait]
impl crate::plan::PmLike for CompletionTimeoutPm {
    async fn list_issues(
        &self,
        _filter: spur_pm::IssueFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        std::future::pending().await
    }

    async fn update_issue(&self, _id: &str, update: spur_pm::IssueUpdate) -> anyhow::Result<()> {
        self.updates
            .lock()
            .expect("updates lock")
            .push(update.clone());
        let mut labels = self.labels.lock().expect("labels lock");
        for remove in update.remove_labels {
            labels.retain(|label| label != &remove);
        }
        for add in update.add_labels {
            if !labels.contains(&add) {
                labels.push(add);
            }
        }
        Ok(())
    }

    async fn issue_labels(&self, _id: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.labels.lock().expect("labels lock").clone())
    }

    fn closed_status(&self) -> &str {
        "closed"
    }

    fn advanced(&self) -> Option<&dyn spur_pm::BeadsAdvanced> {
        Some(&self.advanced)
    }
}

#[tokio::test(start_paused = true)]
async fn completion_collector_project_timeout_delivers_via_deferred_push() {
    let plan_id = "plan-timeout";
    let task_id = "task-timeout";
    let issue_id = "bd-timeout";
    let delegation_id = "del-timeout";
    let attempt = 1;
    let pm = Arc::new(CompletionTimeoutPm::default());
    let feature_gate = pro_feature_gate();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-timeout".into()));
    let materializer = crate::outcome_materializer::OutcomeMaterializer::new(Arc::new(
        spur_blob_store::MemoryOutcomeStore::new(),
    ));
    let result = spur_acp::DelegationResult {
        resolved_config: None,
        status: spur_acp::DelegationStatus::Success,
        diff: None,
        diff_summary: None,
        summary: Some("worker done".to_string()),
        estimated_cost_usd: 0.0,
        worker_branch: Some("spur/worker-timeout".to_string()),
        artifact: None,
    };
    let deferred = crate::plan::persist_worker_completion_and_notify(
        pm.as_ref(),
        issue_id,
        feature_gate.as_ref(),
        plan_id,
        delegation_id,
        &None,
        &result,
        &brain_session_id,
        attempt,
        &materializer,
        None,
        None,
        Some(task_id),
    )
    .await
    .expect("persist completion")
    .expect("successful completion should defer notification");

    let captured = CapturedCompletionCollectorErrors::default();
    let sink = Arc::new(RecordingEventSink::default());
    let continuation_count = Arc::new(AtomicUsize::new(0));
    let continuation_count_for_ctx = Arc::clone(&continuation_count);
    let continuation_ctx = Arc::new(crate::plan::continuation::DetachedContinuationCtx {
        on_complete: Arc::new(move |_, _| {
            let continuation_count = Arc::clone(&continuation_count_for_ctx);
            Box::pin(async move {
                continuation_count.fetch_add(1, Ordering::SeqCst);
            })
        }),
    });
    let outcomes = Arc::new(tokio::sync::Mutex::new(OutcomeStore::default()));
    outcomes.lock().await.record_dispatched(
        plan_id,
        task_id,
        "w",
        delegation_id,
        false,
        SystemTime::UNIX_EPOCH,
    );
    let tracker = crate::server::AbortableTaskTracker::new();
    let dispatch = test_completion_dispatch(tracker.clone(), brain_session_id.to_string());
    let context = CompletionCollectorLogContext {
        plan_id: plan_id.to_string(),
        task_id: task_id.to_string(),
        delegation_id: delegation_id.to_string(),
        brain_session_id: brain_session_id.to_string(),
        attempt,
    };

    let guard = tracing::subscriber::set_default(captured.clone());
    spawn_completion_collector(&dispatch, context, {
        let pm = Arc::clone(&pm);
        let feature_gate = Arc::clone(&feature_gate);
        let sink = Arc::clone(&sink);
        let continuation_ctx = Arc::clone(&continuation_ctx);
        let outcomes = Arc::clone(&outcomes);
        let brain_session_id = brain_session_id.clone();
        async move {
            project_completion_snapshot_and_deliver(
                &SystemClock,
                crate::plan::projector::project_plan_from_beads(
                    pm.as_ref(),
                    plan_id,
                    feature_gate.as_ref(),
                ),
                &outcomes,
                Some(sink.as_ref()),
                continuation_ctx.as_ref(),
                Some(deferred),
                CompletionProjectionLogContext {
                    plan_id,
                    task_id,
                    delegation_id,
                    brain_session_id: &brain_session_id,
                    attempt,
                },
            )
            .await;
        }
    });
    tokio::task::yield_now().await;
    tokio::time::advance(COMPLETION_PROJECTION_TIMEOUT + Duration::from_millis(1)).await;
    tracker.close();
    tokio::time::timeout(Duration::from_secs(12), tracker.wait())
        .await
        .expect("completion collector must return after projection timeout");
    drop(guard);

    assert!(
        captured.contains_event_with(&[
            "project stage timed out",
            "plan_id=plan-timeout",
            "task_id=task-timeout",
            "delegation_id=del-timeout",
            "brain_session_id=brain-timeout",
            "attempt=1",
            "timeout_ms=10000",
        ]),
        "expected timeout error log, got {:?}",
        captured
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|event| format!("{} {}", event.target, event.fields))
            .collect::<Vec<_>>()
    );
    assert!(
        captured.contains_event_with(&["stage_completed_project_timeout"]),
        "expected timeout checkpoint"
    );
    assert!(
        captured.contains_event_with(&["stage_delivered_on_project_timeout"]),
        "expected timeout delivery checkpoint"
    );
    let emitted_events = sink.events.lock().expect("events lock");
    assert!(
        !emitted_events
            .iter()
            .any(|event| matches!(event, spur_acp::SpurEventBody::PlanSnapshotUpdated { .. })),
        "timeout must not emit PlanSnapshotUpdated"
    );
    assert!(
        emitted_events.iter().any(|event| matches!(
            event,
            spur_acp::SpurEventBody::PlanTaskAwaitingReview { .. }
        )),
        "timeout must requeue the deferred plan-task event"
    );
    drop(emitted_events);
    assert_eq!(
        continuation_count.load(Ordering::SeqCst),
        1,
        "timeout must push the deferred completion back onto the continuation queue"
    );
    assert!(
        outcomes
            .lock()
            .await
            .recent_outcomes(plan_id)
            .iter()
            .any(|outcome| matches!(
                outcome,
                DispatchOutcome::Dispatched {
                    task_id: recorded_task_id,
                    delegation_id: recorded_delegation_id,
                    ..
                } if recorded_task_id == task_id && recorded_delegation_id == delegation_id
            )),
        "timeout must leave the outcome row available for the next projection attempt"
    );
    assert!(
        pm.advanced
            .comments
            .lock()
            .expect("comments lock")
            .iter()
            .any(|comment| matches!(
                crate::plan::audit_sentinel::parse_comment(&comment.body),
                Some(Ok(
                    crate::plan::audit_sentinel::AuditSentinelKind::Completion { .. }
                ))
            )),
        "completion audit should be durable before projection timeout"
    );
}

#[tokio::test(start_paused = true)]
async fn completion_collector_project_timeout_without_deferred_warns_and_returns() {
    let plan_id = "plan-timeout-none";
    let task_id = "task-timeout-none";
    let delegation_id = "del-timeout-none";
    let attempt = 1;
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-timeout-none".into()));
    let captured = CapturedCompletionCollectorErrors::default();
    let sink = Arc::new(RecordingEventSink::default());
    let continuation_ctx = Arc::new(crate::plan::continuation::DetachedContinuationCtx {
        on_complete: Arc::new(|_, _| Box::pin(async {})),
    });
    let outcomes = Arc::new(tokio::sync::Mutex::new(OutcomeStore::default()));
    let tracker = crate::server::AbortableTaskTracker::new();
    let dispatch = test_completion_dispatch(tracker.clone(), brain_session_id.to_string());
    let context = CompletionCollectorLogContext {
        plan_id: plan_id.to_string(),
        task_id: task_id.to_string(),
        delegation_id: delegation_id.to_string(),
        brain_session_id: brain_session_id.to_string(),
        attempt,
    };

    let guard = tracing::subscriber::set_default(captured.clone());
    spawn_completion_collector(&dispatch, context, {
        let sink = Arc::clone(&sink);
        let continuation_ctx = Arc::clone(&continuation_ctx);
        let outcomes = Arc::clone(&outcomes);
        let brain_session_id = brain_session_id.clone();
        async move {
            project_completion_snapshot_and_deliver(
                &SystemClock,
                std::future::pending(),
                &outcomes,
                Some(sink.as_ref()),
                continuation_ctx.as_ref(),
                None,
                CompletionProjectionLogContext {
                    plan_id,
                    task_id,
                    delegation_id,
                    brain_session_id: &brain_session_id,
                    attempt,
                },
            )
            .await;
        }
    });
    tokio::task::yield_now().await;
    tokio::time::advance(COMPLETION_PROJECTION_TIMEOUT + Duration::from_millis(1)).await;
    tracker.close();
    tokio::time::timeout(Duration::from_secs(12), tracker.wait())
        .await
        .expect("completion collector must return after projection timeout");
    drop(guard);

    assert!(
        captured.contains_event_with(&["completion lost — no deferred queue to requeue into"]),
        "expected missing-deferred warning"
    );
    assert!(
        sink.events.lock().expect("events lock").is_empty(),
        "timeout without a deferred push must not emit events"
    );
}

#[tokio::test]
async fn projected_plan_for_ready_reuses_hydrated_state_without_projecting() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let hydrated = Arc::new(projected_test_state("hydrated-plan"));
    let fallback_polled = Arc::new(AtomicBool::new(false));
    let fallback_polled_for_future = Arc::clone(&fallback_polled);

    let projected = projected_plan_for_ready(Some(Arc::clone(&hydrated)), async move {
        fallback_polled_for_future.store(true, Ordering::SeqCst);
        Ok(projected_test_state("fallback-plan"))
    })
    .await
    .expect("hydrated plan state should be returned");

    assert!(Arc::ptr_eq(&projected, &hydrated));
    assert!(!fallback_polled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn projected_plan_for_ready_projects_when_unhydrated() {
    let projected =
        projected_plan_for_ready(None, async { Ok(projected_test_state("fallback-plan")) })
            .await
            .expect("fallback projection should be used");

    assert_eq!(projected.plan_id, "fallback-plan");
}

#[test]
fn reconciler_dispatch_ctx_can_be_cloned_for_server_startup() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let ctx = super::ReconcilerDispatchCtx {
        delegation_tx: tx,
        task_tracker: crate::server::AbortableTaskTracker::new(),
        brain_session_id: spur_acp::BrainSessionId::new(spur_acp::SessionId("brain".into())),
        event_sink: None,
        materializer: Arc::new(crate::outcome_materializer::OutcomeMaterializer::new(
            Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
        )),
        continuation_ctx: Arc::new(crate::plan::continuation::DetachedContinuationCtx {
            on_complete: Arc::new(|_, _| Box::pin(async {})),
        }),
    };

    let cloned = ctx.clone();
    assert_eq!(cloned.brain_session_id, ctx.brain_session_id);
}

#[test]
fn prior_branch_for_reuse_uses_last_attempt_only_when_reuse_requested() {
    let task = crate::plan::PlanTaskEntry {
        spec: crate::plan::PlanTask {
            task_id: "T1".into(),
            agent: "codex".into(),
            profile: None,
            skills: None,
            model: None,
            effort: None,
            config_overrides: None,
            task: "Task".into(),
            depends_on: vec![],
            issue_id: Some("bd-1".into()),
            issue_title: None,
            context_files: vec![],
            planned_write_files: None,
        },
        status: crate::plan::PlanTaskStatus::Ready,
        result: None,
        worker_branch: None,
        attempt: 2,
        history: vec![
            crate::plan::AttemptRecord {
                attempt: 1,
                worker_branch: Some("spur/worker-old".into()),
                diff_summary: None,
                summary: None,
                feedback: "first".into(),
                dispatched_base_oid: None,
                reuse_prior_worktree: None,
            },
            crate::plan::AttemptRecord {
                attempt: 2,
                worker_branch: Some("spur/worker-reuse".into()),
                diff_summary: None,
                summary: None,
                feedback: "second".into(),
                dispatched_base_oid: None,
                reuse_prior_worktree: Some(true),
            },
        ],
        last_delegation_id: None,
        dispatched_base_oid: None,
    };

    assert_eq!(
        super::prior_branch_for_reuse(&task),
        Some("spur/worker-reuse".into())
    );
}

fn summary(id: &str, status: &str) -> spur_pm::IssueSummary {
    spur_pm::IssueSummary {
        id: id.into(),
        source: spur_pm::PmSource::Beads,
        title: id.into(),
        status: status.into(),
        labels: vec![],
        url: format!("https://example.invalid/{id}"),
        priority: None,
        issue_type: Some("task".into()),
        assignee: None,
        description: None,
    }
}

#[test]
fn classify_epic_completion_reports_all_approved() {
    let children = vec![summary("bd-1", "closed"), summary("bd-2", "closed")];
    let outcome = super::terminal::classify_epic_completion(&children, "closed").expect("terminal");
    assert_eq!(
        outcome.audit_outcome,
        crate::plan::audit_sentinel::EpicCompletionOutcome::AllApproved
    );
    assert!(outcome.add_integration_pending);
}

#[test]
fn classify_epic_completion_reports_terminal_failures() {
    let mut rejected = summary("bd-2", "closed");
    rejected.labels.push("rejected".into());
    let children = vec![summary("bd-1", "closed"), rejected];
    let outcome = super::terminal::classify_epic_completion(&children, "closed").expect("terminal");
    assert_eq!(
        outcome.audit_outcome,
        crate::plan::audit_sentinel::EpicCompletionOutcome::TerminalWithFailures
    );
    assert!(!outcome.add_integration_pending);
}

#[test]
fn build_loop_run_sums_costs_and_derives_partial_outcome() {
    use crate::plan::loops::run_record::{
        build_loop_run, sum_completion_cost_micros, LoopRunOutcome,
    };

    let audits = vec![
        crate::plan::audit_sentinel::AuditSentinelKind::Completion {
            delegation_id: "del-a".into(),
            completion_state: crate::plan::audit_sentinel::CompletionState::AwaitingReview,
            superseded: false,
            worker_branch: None,
            result_summary: None,
            artifact_uri: None,
            dispatched_base_oid: None,
            estimated_cost_micros: Some(300),
        },
        crate::plan::audit_sentinel::AuditSentinelKind::Completion {
            delegation_id: "del-b".into(),
            completion_state: crate::plan::audit_sentinel::CompletionState::AwaitingReview,
            superseded: false,
            worker_branch: None,
            result_summary: None,
            artifact_uri: None,
            dispatched_base_oid: None,
            estimated_cost_micros: Some(500),
        },
        crate::plan::audit_sentinel::AuditSentinelKind::Completion {
            delegation_id: "del-legacy".into(),
            completion_state: crate::plan::audit_sentinel::CompletionState::AwaitingReview,
            superseded: false,
            worker_branch: None,
            result_summary: None,
            artifact_uri: None,
            dispatched_base_oid: None,
            estimated_cost_micros: None,
        },
        crate::plan::audit_sentinel::AuditSentinelKind::EscalationRequested {
            plan_id: "P1".into(),
            task_id: "T3".into(),
            attempt: 2,
            last_error: "needs brain".into(),
            worker_branch: None,
            delegation_id: Some("del-c".into()),
        },
    ];

    assert_eq!(sum_completion_cost_micros(&audits), 800);

    let run = build_loop_run(
        "loopA",
        7,
        "P1",
        Some("l2".into()),
        LoopRunOutcome {
            tasks_discovered: 3,
            approved: 2,
            rejected: 0,
            failed: 1,
            cancelled: 0,
        },
        &audits,
        1_782_953_600,
    );

    assert_eq!(
        run,
        crate::plan::audit_sentinel::AuditSentinelKind::LoopRun {
            loop_id: "loopA".into(),
            generation: 7,
            plan_id: "P1".into(),
            autonomy: Some("l2".into()),
            outcome: "partial".into(),
            tasks_discovered: 3,
            approved: 2,
            rejected: 0,
            failed: 1,
            cancelled: 0,
            escalations: 1,
            cost_micros: 800,
            started_at: 1_782_953_600,
            ended_at: 1_782_953_600,
        }
    );
}

/// D1 fix coverage: verify that the biased select! pattern used inside
/// `Reconciler::run` to race `tick_once` against `cancel` actually
/// preempts an in-flight future when cancel fires. Uses a pending future
/// as a stand-in for a stuck `bv.triage`/`br ready` call; without the
/// biased cancel race, the task would hang indefinitely.
#[tokio::test]
async fn biased_select_cancel_preempts_pending_tick() {
    use std::future::pending;
    use tokio::sync::oneshot;

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    tokio::pin!(cancel_rx);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let _ = cancel_tx.send(());
    });

    let blocking = pending::<anyhow::Result<bool>>();
    tokio::pin!(blocking);

    let outcome = tokio::time::timeout(Duration::from_secs(1), async move {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => "cancelled",
            _ = &mut blocking => "tick_completed",
        }
    })
    .await
    .expect("select must not hang when cancel is live");

    assert_eq!(outcome, "cancelled");
}

#[test]
fn cadence_backoff_formula() {
    let cfg = ReconcilerConfig {
        base_interval: Duration::from_secs(1),
        idle_ceiling: Duration::from_secs(8),
        backoff_factor: 2,
        ..Default::default()
    };
    let mut d = cfg.base_interval;
    let mut hist = vec![d];
    for _ in 0..5 {
        d = std::cmp::min(d.saturating_mul(cfg.backoff_factor), cfg.idle_ceiling);
        hist.push(d);
    }
    assert_eq!(
        hist,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(8),
            Duration::from_secs(8),
        ]
    );
}

/// Regression: journal monitor must exit promptly when aborted so that
/// graceful shutdown does not hang forever awaiting the handle and so
/// that abort/drop does not leak a detached polling task.
#[tokio::test]
async fn journal_monitor_exits_on_abort_without_hang() {
    use std::time::Duration;

    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("journal");
    tokio::fs::write(&path, b"x").await.expect("write");
    let notify = Arc::new(Notify::new());
    let handle = tokio::spawn(monitor_journal_appends(path, notify));
    handle.abort();
    let result = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("monitor must exit within 1s of abort");
    assert!(
        result.is_err() && result.unwrap_err().is_cancelled(),
        "monitor must be cancelled, not panic"
    );
}

#[tokio::test]
async fn monitor_journal_appends_survives_transient_metadata_error() {
    use std::time::Duration;

    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("journal");
    let hidden = dir.path().join("journal.hidden");
    tokio::fs::write(&path, b"seed")
        .await
        .expect("write seed journal");

    let notify = Arc::new(Notify::new());
    let handle = tokio::spawn(monitor_journal_appends(path.clone(), Arc::clone(&notify)));

    tokio::time::sleep(Duration::from_millis(50)).await;
    tokio::fs::rename(&path, &hidden)
        .await
        .expect("hide journal to force metadata failure");
    tokio::time::sleep(Duration::from_millis(350)).await;
    tokio::fs::write(&path, b"seed-after-retry")
        .await
        .expect("recreate journal with appended content");

    tokio::time::timeout(Duration::from_secs(2), notify.notified())
        .await
        .expect("monitor should retry transient metadata failures and wake on later append");

    handle.abort();
    let _ = handle.await;
}

#[test]
fn auto_pr_params_include_plan_id_and_summary() {
    let params = super::terminal::build_auto_pr_params(
        "plan-123",
        "Epic title",
        "All approved",
        "spur/merge-1",
    );
    assert!(
        params.title.contains("plan-123"),
        "title missing plan_id: {}",
        params.title
    );
    assert!(
        params.body.contains("All approved"),
        "body missing outcome: {}",
        params.body
    );
    assert_eq!(params.head_branch, "spur/merge-1");
}

struct MockAutomation {
    actions: Arc<tokio::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl super::ReconcilerAutomation for MockAutomation {
    async fn merge_plan(&self, plan_id: &str) -> anyhow::Result<crate::plan::PlanMergeState> {
        self.actions.lock().await.push(format!("merge:{plan_id}"));
        Ok(crate::plan::PlanMergeState::Succeeded {
            merge_branch: "spur/merge-1".to_string(),
            merged_task_ids: vec![],
        })
    }

    async fn create_pr(&self, params: spur_pm::PrParams) -> anyhow::Result<String> {
        self.actions
            .lock()
            .await
            .push(format!("pr:{}", params.title));
        Ok("https://example.invalid/pr/1".to_string())
    }
}

fn attach_beads_workspace(repo: &std::path::Path, w: &spur_pm::test_workspace::TestBeadsWorkspace) {
    let beads_dir = repo.join(".beads");
    std::fs::create_dir(&beads_dir).expect("create test .beads directory");
    // Copy db + WAL + SHM (beads_rust uses WAL mode and skips checkpoint
    // on Drop; bare `fs::copy(beads.db)` loses every uncheckpointed write).
    w.copy_db_to(&beads_dir);
}

fn workspace_with_complete_epic(
    repo: &std::path::Path,
    plan_id: &str,
) -> spur_pm::test_workspace::TestBeadsWorkspace {
    let mut w = spur_pm::test_workspace::TestBeadsWorkspace::init();
    let plan_id_label = format!("spur:plan-id:{plan_id}");
    let epic_id = w.create_epic("Test Epic");

    for title in ["Task A", "Task B"] {
        let task_id = w.create_issue(title);
        w.add_label(&task_id, &plan_id_label);
        w.close_issue(&task_id);
    }

    w.add_label(&epic_id, &plan_id_label);
    w.add_label(&epic_id, "spur:plan-complete");
    w.close_issue(&epic_id);
    w.add_label(&epic_id, "spur:integration-pending");
    attach_beads_workspace(repo, &w);
    w
}

async fn pm_for_beads_repo(repo: &std::path::Path) -> Arc<spur_pm::PmService> {
    Arc::new(
        spur_pm::PmService::try_new(None, true, false, repo, None)
            .await
            .expect("PmService::try_new failed")
            .expect("expected beads pm"),
    )
}

fn run_git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("git command failed to start");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout should be utf-8")
        .trim()
        .to_string()
}

fn seed_git_repo(repo: &std::path::Path) -> String {
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.email", "test@spur"]);
    run_git(repo, &["config", "user.name", "spur-test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").expect("write seed file");
    run_git(repo, &["add", "seed.txt"]);
    run_git(repo, &["commit", "-q", "-m", "seed"]);
    run_git(repo, &["rev-parse", "HEAD"])
}

fn create_worker_branch(repo: &std::path::Path, branch: &str, file: &str) -> String {
    run_git(repo, &["checkout", "-q", "main"]);
    run_git(repo, &["checkout", "-q", "-b", branch]);
    std::fs::write(repo.join(file), format!("{branch}\n")).expect("write worker file");
    run_git(repo, &["add", file]);
    run_git(repo, &["commit", "-q", "-m", branch]);
    run_git(repo, &["checkout", "-q", "main"]);
    branch.to_string()
}

fn test_dispatch_ctx(
    delegation_tx: tokio::sync::mpsc::Sender<crate::DelegationRequest>,
    brain_session_id: spur_acp::BrainSessionId,
) -> ReconcilerDispatchCtx {
    ReconcilerDispatchCtx {
        delegation_tx,
        task_tracker: crate::server::AbortableTaskTracker::new(),
        brain_session_id,
        event_sink: None,
        materializer: Arc::new(crate::outcome_materializer::OutcomeMaterializer::new(
            Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
        )),
        continuation_ctx: Arc::new(crate::plan::continuation::DetachedContinuationCtx {
            on_complete: Arc::new(|_, _| Box::pin(async {})),
        }),
    }
}

fn test_dispatch_ctx_with_recording(
    delegation_tx: tokio::sync::mpsc::Sender<crate::DelegationRequest>,
    brain_session_id: spur_acp::BrainSessionId,
    event_sink: Option<Arc<dyn spur_mcp::events::McpEventSink>>,
) -> (
    ReconcilerDispatchCtx,
    Arc<std::sync::Mutex<Vec<spur_acp::domain::BrainContinuation>>>,
) {
    let continuations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = Arc::clone(&continuations);
    (
        ReconcilerDispatchCtx {
            delegation_tx,
            task_tracker: crate::server::AbortableTaskTracker::new(),
            brain_session_id,
            event_sink,
            materializer: Arc::new(crate::outcome_materializer::OutcomeMaterializer::new(
                Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
            )),
            continuation_ctx: Arc::new(crate::plan::continuation::DetachedContinuationCtx {
                on_complete: Arc::new(move |continuation, _worker_session| {
                    let captured = Arc::clone(&captured);
                    Box::pin(async move {
                        captured
                            .lock()
                            .expect("continuations lock")
                            .push(continuation);
                    })
                }),
            }),
        },
        continuations,
    )
}

struct ScriptedReadyPm {
    inner: Arc<crate::plan::test_util::MockPm>,
    empty_plan_ids: HashSet<String>,
}

#[async_trait::async_trait]
impl crate::plan::PmLike for ScriptedReadyPm {
    async fn get_issue(&self, id: &str) -> anyhow::Result<spur_pm::Issue> {
        self.inner.get_issue(id).await
    }

    async fn list_issues(
        &self,
        filter: spur_pm::IssueFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        self.inner.list_issues(filter).await
    }

    async fn create_issue(&self, params: spur_pm::IssueCreate) -> anyhow::Result<String> {
        self.inner.create_issue(params).await
    }

    async fn update_issue(&self, id: &str, update: spur_pm::IssueUpdate) -> anyhow::Result<()> {
        self.inner.update_issue(id, update).await
    }

    async fn update_issues_atomically(
        &self,
        idempotency_key: &str,
        preconditions: Vec<spur_pm::AtomicUpdatePrecondition>,
        updates: Vec<(String, spur_pm::IssueUpdate)>,
    ) -> anyhow::Result<spur_pm::AtomicUpdateOutcome> {
        self.inner
            .update_issues_atomically(idempotency_key, preconditions, updates)
            .await
    }

    async fn add_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        self.inner.add_dependency(issue_id, depends_on_id).await
    }

    async fn issue_labels(&self, id: &str) -> anyhow::Result<Vec<String>> {
        self.inner.issue_labels(id).await
    }

    fn closed_status(&self) -> &str {
        self.inner.closed_status()
    }

    fn source_str(&self) -> &'static str {
        self.inner.source_str()
    }

    fn issue_graph_available(&self) -> bool {
        self.inner.issue_graph_available()
    }

    async fn issue_subgraph_json(
        &self,
        id: &str,
    ) -> anyhow::Result<spur_pm::graph::DependencyGraph> {
        self.inner.issue_subgraph_json(id).await
    }

    fn advanced(&self) -> Option<&dyn spur_pm::BeadsAdvanced> {
        Some(self)
    }
}

#[async_trait::async_trait]
impl spur_pm::BeadsAdvanced for ScriptedReadyPm {
    async fn list_ready(
        &self,
        filter: spur_pm::ReadyFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        if filter.labels_all.iter().any(|label| {
            crate::plan::labels::parse_plan_id(label)
                .is_some_and(|plan_id| self.empty_plan_ids.contains(plan_id))
        }) {
            return Ok(Vec::new());
        }
        spur_pm::BeadsAdvanced::list_ready(self.inner.as_ref(), filter).await
    }

    async fn list_comments(&self, issue_id: &str) -> anyhow::Result<Vec<spur_pm::Comment>> {
        spur_pm::BeadsAdvanced::list_comments(self.inner.as_ref(), issue_id).await
    }

    async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<String> {
        spur_pm::BeadsAdvanced::add_comment(self.inner.as_ref(), issue_id, body).await
    }

    async fn remove_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        spur_pm::BeadsAdvanced::remove_dependency(self.inner.as_ref(), issue_id, depends_on_id)
            .await
    }

    async fn dep_cycles(&self) -> anyhow::Result<Vec<spur_pm::DependencyCycle>> {
        spur_pm::BeadsAdvanced::dep_cycles(self.inner.as_ref()).await
    }
}

async fn seed_mock_complete_epic(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    brain_session_id: &spur_acp::BrainSessionId,
) -> String {
    seed_mock_complete_epic_for_owner(pm, plan_id, brain_session_id.as_session_id()).await
}

async fn seed_mock_complete_epic_for_owner(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    owner_session_id: &spur_acp::SessionId,
) -> String {
    pm.create_issue(spur_pm::IssueCreate {
        title: format!("{plan_id}: epic"),
        description: Some(format!("{plan_id} test epic")),
        issue_type: Some("epic".into()),
        labels: vec![
            crate::plan::labels::plan_id(plan_id),
            crate::plan::labels::PLAN_COMPLETE.to_string(),
            crate::plan::labels::plan_owner(&owner_session_id.0),
        ],
        ..Default::default()
    })
    .await
    .expect("create mock complete epic")
}

async fn seed_mock_ready_task_plan(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    task_id: &str,
    brain_session_id: &spur_acp::BrainSessionId,
) -> String {
    seed_mock_ready_tasks_plan(pm, plan_id, &[task_id], brain_session_id)
        .await
        .into_iter()
        .next()
        .expect("seeded one task")
}

async fn seed_mock_ready_tasks_plan(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    task_ids: &[&str],
    brain_session_id: &spur_acp::BrainSessionId,
) -> Vec<String> {
    let epic_id = seed_mock_complete_epic(pm, plan_id, brain_session_id).await;
    let mut issue_ids = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        let issue_id = pm
            .create_issue(spur_pm::IssueCreate {
                title: format!("{task_id}: ready task"),
                description: Some(format!("{task_id} test task")),
                issue_type: Some("task".into()),
                labels: vec![
                    crate::plan::labels::plan_id(plan_id),
                    crate::plan::labels::plan_task_id(task_id),
                    crate::plan::labels::agent("codex"),
                ],
                parent: Some(epic_id.clone()),
                ..Default::default()
            })
            .await
            .expect("create mock ready task");
        issue_ids.push(issue_id);
    }

    let adv =
        crate::plan::PmLike::advanced(pm.as_ref()).expect("mock pm should expose beads advanced");
    adv.add_comment(
        &epic_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::PlanSubmit {
                plan_id: plan_id.to_string(),
                epic_issue_id: epic_id.clone(),
                task_ids: issue_ids.clone(),
                base_snapshot_branch: Some("main".to_string()),
                base_snapshot_oid: None,
                execution_mode: None,
                brain_session_id: Some(brain_session_id.as_session_id().0.clone()),
                explicit_base: None,
            },
        ),
    )
    .await
    .expect("plan submit audit");
    for (issue_id, task_id) in issue_ids.iter().zip(task_ids) {
        crate::plan::emit_task_spec_audit(
            adv,
            issue_id,
            task_id,
            "codex",
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
        )
        .await
        .expect("task spec audit");
    }

    issue_ids
}

async fn add_mock_issue_label(
    pm: &Arc<crate::plan::test_util::MockPm>,
    issue_id: &str,
    label: impl Into<String>,
) {
    pm.update_issue(
        issue_id,
        spur_pm::IssueUpdate {
            add_labels: vec![label.into()],
            ..Default::default()
        },
    )
    .await
    .expect("add mock issue label");
}

async fn add_mock_epic_label(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    label: impl Into<String>,
) {
    let epics = pm
        .list_issues(spur_pm::IssueFilter {
            labels: vec![crate::plan::labels::plan_id(plan_id)],
            issue_type: Some("epic".into()),
            ..Default::default()
        })
        .await
        .expect("list mock epics");
    let epic_id = epics
        .first()
        .unwrap_or_else(|| panic!("expected complete epic for plan {plan_id}"))
        .id
        .clone();
    add_mock_issue_label(pm, &epic_id, label).await;
}

async fn close_mock_task_with_completion_cost(
    pm: &Arc<crate::plan::test_util::MockPm>,
    issue_id: &str,
    estimated_cost_micros: u64,
) {
    let adv =
        crate::plan::PmLike::advanced(pm.as_ref()).expect("mock pm should expose beads advanced");
    adv.add_comment(
        issue_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::Completion {
                delegation_id: "del-budget-spent".to_string(),
                completion_state: crate::plan::audit_sentinel::CompletionState::AwaitingReview,
                superseded: false,
                worker_branch: Some("spur/worker-budget".to_string()),
                result_summary: None,
                artifact_uri: None,
                dispatched_base_oid: None,
                estimated_cost_micros: Some(estimated_cost_micros),
            },
        ),
    )
    .await
    .expect("completion audit");
    adv.add_comment(
        issue_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::Approval {
                delegation_id: "del-budget-spent".to_string(),
            },
        ),
    )
    .await
    .expect("approval audit");
    pm.update_issue(
        issue_id,
        spur_pm::IssueUpdate {
            status: Some("closed".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("close spent task");
}

async fn create_mock_loop_issue(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_id: &str,
    autonomy: crate::plan::loops::spec::AutonomyLevel,
    template: serde_json::Value,
    max_cost_micros_per_generation: Option<u64>,
) -> String {
    let spec = crate::plan::loops::spec::LoopSpec {
        loop_id: loop_id.to_string(),
        goal: format!("Goal for {loop_id}"),
        pattern: None,
        cadence_secs: 60,
        autonomy,
        template,
        governors: crate::plan::loops::spec::LoopGovernors {
            max_cost_micros_per_generation,
            ..Default::default()
        },
        escalation: None,
    };
    pm.create_issue(spur_pm::IssueCreate {
        title: format!("Loop {loop_id}"),
        description: Some(spec.to_sentinel_body()),
        issue_type: Some(crate::plan::loops::LOOP_ISSUE_TYPE.to_string()),
        labels: vec![
            crate::plan::labels::loop_id_label(loop_id),
            format!(
                "{}{}",
                crate::plan::labels::AUTONOMY_PREFIX,
                autonomy.as_str()
            ),
        ],
        ..Default::default()
    })
    .await
    .expect("create loop issue")
}

#[tokio::test(start_paused = true)]
async fn terminal_loop_epic_appends_one_loop_run_to_loop_issue() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-loop-run".into()));
    let loop_id = "loopA";
    let loop_issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "Loop controller".into(),
            description: Some("loop issue".into()),
            issue_type: Some(crate::plan::loops::LOOP_ISSUE_TYPE.to_string()),
            labels: vec![crate::plan::labels::loop_id_label(loop_id)],
            ..Default::default()
        })
        .await
        .expect("create loop issue");
    let task_issue_ids =
        seed_mock_ready_tasks_plan(&pm, "P-LOOP-RUN", &["T1", "T2"], &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-LOOP-RUN",
        crate::plan::labels::loop_id_label(loop_id),
    )
    .await;
    add_mock_epic_label(
        &pm,
        "P-LOOP-RUN",
        crate::plan::labels::loop_generation_label(1),
    )
    .await;
    add_mock_epic_label(
        &pm,
        "P-LOOP-RUN",
        format!("{}l2", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    close_mock_task_with_completion_cost(&pm, &task_issue_ids[0], 300).await;
    close_mock_task_with_completion_cost(&pm, &task_issue_ids[1], 500).await;

    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let mut reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone(),
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );
    reconciler.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(12_345),
    }));

    reconciler.tick_once().await.expect("first tick");
    reconciler.tick_once().await.expect("second tick");

    let loop_comments = pm.comments(&loop_issue_id).await;
    let loop_runs = loop_comments
        .iter()
        .filter_map(|comment| crate::plan::audit_sentinel::parse_comment(&comment.body))
        .filter_map(Result::ok)
        .filter(|audit| {
            matches!(
                audit,
                crate::plan::audit_sentinel::AuditSentinelKind::LoopRun {
                    loop_id: found_loop_id,
                    generation: 1,
                    ..
                } if found_loop_id == loop_id
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(loop_runs.len(), 1, "LoopRun must be idempotent");
    assert_eq!(
        loop_runs[0],
        crate::plan::audit_sentinel::AuditSentinelKind::LoopRun {
            loop_id: loop_id.into(),
            generation: 1,
            plan_id: "P-LOOP-RUN".into(),
            autonomy: Some("l2".into()),
            outcome: "approved".into(),
            tasks_discovered: 2,
            approved: 2,
            rejected: 0,
            failed: 0,
            cancelled: 0,
            escalations: 0,
            cost_micros: 800,
            started_at: 12_345,
            ended_at: 12_345,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn system_l3_runtime_arms_generation_without_brain() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let loop_id = "loopl3arm";
    let triage_body = "Triage the stored template\nkeep this body byte-exact";
    let action_body = "Apply the approved action\nsecond line stays byte-exact";
    let template = serde_json::json!({
        "epic_title": "Stored L3 generation",
        "epic_body": "Generated from stored loop template",
        "tasks": [
            {
                "task_id": "Triage",
                "agent": "codex",
                "task": triage_body,
                "context_files": ["docs/loop.md"]
            },
            {
                "task_id": "Action",
                "agent": "codex",
                "task": action_body,
                "depends_on": ["Triage"]
            }
        ]
    });
    create_mock_loop_issue(
        &pm,
        loop_id,
        crate::plan::loops::spec::AutonomyLevel::L3,
        template,
        Some(123_456),
    )
    .await;
    let brain_session_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(
        crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
    ));
    let event_sink = Arc::new(RecordingEventSink::default());
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel::<crate::DelegationRequest>(4);
    let (dispatch, continuations) =
        test_dispatch_ctx_with_recording(delegation_tx, brain_session_id, Some(event_sink.clone()));
    let mut reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig {
            loop_sweep_scope: crate::plan::loops::LoopSweepScope::L3Only,
            plan_scope: PlanScope::SystemL3Only,
            ..Default::default()
        },
        pm.clone(),
        Arc::new(Notify::new()),
        Some(dispatch.into_dispatch()),
        None,
        pro_feature_gate(),
    );
    reconciler.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
    }));

    reconciler.tick_once().await.expect("tick once");

    assert!(
        continuations.lock().expect("continuations lock").is_empty(),
        "L3 should persist directly without a LoopDue continuation"
    );
    let issues = pm.issues().await;
    let generation_epics = issues
        .iter()
        .filter(|issue| {
            issue.issue_type.as_deref() == Some("epic")
                && issue
                    .labels
                    .iter()
                    .any(|label| label == &crate::plan::labels::loop_id_label(loop_id))
        })
        .collect::<Vec<_>>();
    assert_eq!(generation_epics.len(), 1, "expected one L3 generation epic");
    let epic = generation_epics[0];
    for expected in [
        crate::plan::labels::loop_id_label(loop_id),
        crate::plan::labels::loop_generation_label(1),
        format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX),
        format!("{}123456", crate::plan::labels::LOOP_BUDGET_MICROS_PREFIX),
        crate::plan::labels::plan_owner(crate::plan::loops::LOOP_RUNTIME_OWNER_ID),
    ] {
        assert!(
            epic.labels.contains(&expected),
            "generation epic labels missing {expected}: {:?}",
            epic.labels
        );
    }

    let generation_graph = pm
        .issue_subgraph_json(&epic.id)
        .await
        .expect("generation structural graph");
    let child_ids = generation_graph
        .adjacency
        .expect("generation graph adjacency")
        .edges
        .expect("generation graph edges")
        .into_iter()
        .filter(|edge| edge.from == epic.id && edge.edge_type.as_deref() == Some("parent-child"))
        .map(|edge| edge.to)
        .collect::<HashSet<_>>();
    let child_bodies = issues
        .iter()
        .filter(|issue| {
            issue.issue_type.as_deref() == Some("task") && child_ids.contains(&issue.id)
        })
        .filter_map(|issue| {
            let task_id = issue
                .labels
                .iter()
                .find_map(|label| crate::plan::labels::parse_plan_task_id(label))?;
            Some((task_id, issue.body.clone()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        child_bodies
            .get("Triage")
            .expect("triage task body")
            .as_bytes(),
        triage_body.as_bytes()
    );
    assert_eq!(
        child_bodies
            .get("Action")
            .expect("action task body")
            .as_bytes(),
        action_body.as_bytes()
    );
    let events = event_sink.events.lock().expect("events lock");
    assert!(
        events.iter().any(|event| matches!(
            event,
            spur_acp::SpurEventBody::LoopGenerationStarted {
                loop_id: found_loop_id,
                generation: 1,
                ..
            } if found_loop_id == loop_id
        )),
        "expected LoopGenerationStarted event, got {events:?}"
    );
}

async fn seed_system_l3_awaiting_review_plan(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    task_ids: &[&str],
) -> (Vec<String>, String, String) {
    let system_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(
        crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
    ));
    let issue_ids = seed_mock_ready_tasks_plan(pm, plan_id, task_ids, &system_id).await;
    add_mock_epic_label(
        pm,
        plan_id,
        format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    if issue_ids.len() > 1 {
        pm.add_dependency(&issue_ids[1], &issue_ids[0])
            .await
            .expect("add dependent task edge");
    }

    let maker_delegation_id = "maker-del-1".to_string();
    let maker_branch = "spur/worker/maker-del-1".to_string();
    crate::plan::persist_dispatch_intent(
        pm.as_ref(),
        &issue_ids[0],
        pro_feature_gate().as_ref(),
        plan_id,
        &maker_delegation_id,
        "codex",
        1,
        Duration::from_secs(60),
    )
    .await
    .expect("persist maker dispatch");
    let result = spur_acp::DelegationResult {
        resolved_config: None,
        status: spur_acp::DelegationStatus::Success,
        diff: Some("diff --git a/result b/result".into()),
        diff_summary: None,
        summary: Some("maker completed acceptance criteria".into()),
        estimated_cost_usd: 0.0,
        worker_branch: Some(maker_branch.clone()),
        artifact: None,
    };
    let materializer = crate::outcome_materializer::OutcomeMaterializer::new(Arc::new(
        spur_blob_store::MemoryOutcomeStore::new(),
    ));
    crate::plan::persist_worker_completion_and_notify(
        pm.as_ref(),
        &issue_ids[0],
        pro_feature_gate().as_ref(),
        plan_id,
        &maker_delegation_id,
        &None,
        &result,
        &system_id,
        1,
        &materializer,
        None,
        None,
        Some(task_ids[0]),
    )
    .await
    .expect("persist maker completion");

    (issue_ids, maker_delegation_id, maker_branch)
}

fn system_l3_review_reconciler(
    pm: Arc<dyn crate::plan::PmLike>,
) -> (
    Reconciler,
    tokio::sync::mpsc::Receiver<crate::DelegationRequest>,
) {
    let system_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(
        crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
    ));
    let (delegation_tx, delegation_rx) = tokio::sync::mpsc::channel(16);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig {
            loop_sweep_scope: crate::plan::loops::LoopSweepScope::L3Only,
            plan_scope: PlanScope::SystemL3Only,
            predispatch_preview: PreviewStrategy::AlwaysClean,
            dispatch_lease_duration: Duration::from_secs(5),
            ..Default::default()
        },
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, system_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );
    (reconciler, delegation_rx)
}

struct FailFirstCompanionClosePm {
    inner: Arc<crate::plan::test_util::MockPm>,
    fail_close: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl crate::plan::PmLike for FailFirstCompanionClosePm {
    async fn get_issue(&self, id: &str) -> anyhow::Result<spur_pm::Issue> {
        self.inner.get_issue(id).await
    }

    async fn list_issues(
        &self,
        filter: spur_pm::IssueFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        self.inner.list_issues(filter).await
    }

    async fn create_issue(&self, params: spur_pm::IssueCreate) -> anyhow::Result<String> {
        self.inner.create_issue(params).await
    }

    async fn update_issue(&self, id: &str, update: spur_pm::IssueUpdate) -> anyhow::Result<()> {
        let closes_companion = update.status.as_deref() == Some(self.closed_status())
            && self
                .inner
                .get_issue(id)
                .await?
                .labels
                .contains(&crate::plan::labels::SYSTEM_REVIEW.to_string());
        if closes_companion
            && self
                .fail_close
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            anyhow::bail!("injected review companion close failure");
        }
        self.inner.update_issue(id, update).await
    }

    async fn update_issues_atomically(
        &self,
        idempotency_key: &str,
        preconditions: Vec<spur_pm::AtomicUpdatePrecondition>,
        updates: Vec<(String, spur_pm::IssueUpdate)>,
    ) -> anyhow::Result<spur_pm::AtomicUpdateOutcome> {
        self.inner
            .update_issues_atomically(idempotency_key, preconditions, updates)
            .await
    }

    async fn add_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        self.inner.add_dependency(issue_id, depends_on_id).await
    }

    async fn issue_labels(&self, id: &str) -> anyhow::Result<Vec<String>> {
        self.inner.issue_labels(id).await
    }

    fn closed_status(&self) -> &str {
        self.inner.closed_status()
    }

    fn issue_graph_available(&self) -> bool {
        self.inner.issue_graph_available()
    }

    async fn issue_subgraph_json(
        &self,
        id: &str,
    ) -> anyhow::Result<spur_pm::graph::DependencyGraph> {
        self.inner.issue_subgraph_json(id).await
    }

    fn advanced(&self) -> Option<&dyn spur_pm::BeadsAdvanced> {
        Some(self)
    }
}

#[async_trait::async_trait]
impl spur_pm::BeadsAdvanced for FailFirstCompanionClosePm {
    async fn list_ready(
        &self,
        filter: spur_pm::ReadyFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        spur_pm::BeadsAdvanced::list_ready(self.inner.as_ref(), filter).await
    }

    async fn list_comments(&self, issue_id: &str) -> anyhow::Result<Vec<spur_pm::Comment>> {
        spur_pm::BeadsAdvanced::list_comments(self.inner.as_ref(), issue_id).await
    }

    async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<String> {
        spur_pm::BeadsAdvanced::add_comment(self.inner.as_ref(), issue_id, body).await
    }

    async fn remove_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        spur_pm::BeadsAdvanced::remove_dependency(self.inner.as_ref(), issue_id, depends_on_id)
            .await
    }

    async fn dep_cycles(&self) -> anyhow::Result<Vec<spur_pm::DependencyCycle>> {
        spur_pm::BeadsAdvanced::dep_cycles(self.inner.as_ref()).await
    }
}

async fn next_review_request(
    requests: &mut tokio::sync::mpsc::Receiver<crate::DelegationRequest>,
    context: &str,
) -> crate::DelegationRequest {
    tokio::time::timeout(Duration::from_secs(1), requests.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {context}"))
        .unwrap_or_else(|| panic!("delegation channel closed waiting for {context}"))
}

async fn submit_test_review_verdict(
    pm: &crate::plan::test_util::MockPm,
    target_issue_id: &str,
    reviewer_delegation_id: &str,
    decision: &str,
) {
    crate::mcp::review_verdict::submit_review_verdict(
        pm,
        &crate::handlers::WorkerCallContext {
            delegation_id: reviewer_delegation_id.into(),
            brain_session_id: crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
        },
        serde_json::json!({
            "target_issue_id": target_issue_id,
            "decision": decision,
            "feedback": format!("reviewer selected {decision} after inspecting the diff"),
            "evidence": ["get_task_diff and acceptance criteria inspected"]
        }),
    )
    .await
    .expect("submit authenticated review verdict");
}

async fn add_review_signal(
    pm: &crate::plan::test_util::MockPm,
    issue_id: &str,
    delegation_id: &str,
    signal: &crate::plan::signals::WorkerSignal,
) {
    crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        pro_feature_gate().as_ref(),
    )
    .expect("pro gate");
    let (severity, reason) = match signal {
        crate::plan::signals::WorkerSignal::ScopeDrift {
            severity, reason, ..
        }
        | crate::plan::signals::WorkerSignal::Blocked {
            severity, reason, ..
        }
        | crate::plan::signals::WorkerSignal::Risk {
            severity, reason, ..
        } => (*severity, reason.clone()),
        crate::plan::signals::WorkerSignal::Escalate { reason, .. }
        | crate::plan::signals::WorkerSignal::MarkNoop { reason, .. } => (0.0, reason.clone()),
        crate::plan::signals::WorkerSignal::RetryExhausted { .. }
        | crate::plan::signals::WorkerSignal::PotentialClobber { .. } => (0.0, String::new()),
    };
    pm.advanced()
        .expect("beads advanced")
        .add_comment(
            issue_id,
            &crate::plan::audit_sentinel::encode_comment(
                &crate::plan::audit_sentinel::AuditSentinelKind::Signal {
                    signal_id: signal.signal_id().to_string(),
                    delegation_id: delegation_id.to_string(),
                    kind: signal.kind_label().to_string(),
                    severity,
                    reason,
                },
            ),
        )
        .await
        .expect("add attributed signal audit");
    pm.advanced()
        .expect("beads advanced")
        .add_comment(issue_id, &crate::plan::signals::encode_comment(signal))
        .await
        .expect("add signal comment");
    pm.update_issue(
        issue_id,
        spur_pm::IssueUpdate {
            add_labels: vec![crate::plan::labels::signal_kind(signal.kind_label())],
            ..Default::default()
        },
    )
    .await
    .expect("add signal label");
}

#[tokio::test]
async fn system_l3_review_dispatch_is_distinct_durable_and_replayed_once() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, maker_id, maker_branch) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-DISPATCH", &["T1"]).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());

    reconciler.tick_once().await.expect("dispatch reviewer");
    let review = next_review_request(&mut requests, "reviewer request").await;

    assert_ne!(review.id.as_str(), maker_id);
    assert_eq!(
        review.base,
        Some(crate::BaseSpec::Branch { name: maker_branch })
    );
    assert_ne!(review.issue_id.as_deref(), Some(issues[0].as_str()));
    let audits = crate::plan::projector::collect_sorted_audits_for_issue(
        &issues[0],
        pm.comments(&issues[0]).await,
    )
    .expect("target audits");
    assert!(audits.iter().any(|audit| matches!(
        audit,
        crate::plan::audit_sentinel::AuditSentinelKind::SystemReviewDispatch {
            maker_delegation_id,
            reviewer_delegation_id,
            review_issue_id,
            ..
        } if maker_delegation_id == "maker-del-1"
            && reviewer_delegation_id == review.id.as_str()
            && Some(review_issue_id.as_str()) == review.issue_id.as_deref()
    )));

    reconciler.tick_once().await.expect("replay tick");
    assert!(
        requests.try_recv().is_err(),
        "replay must not duplicate review"
    );
}

#[tokio::test]
async fn system_l3_review_mark_noop_reaches_reviewer_but_blocking_signal_does_not() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-NOOP", &["T1"]).await;
    add_review_signal(
        pm.as_ref(),
        &issues[0],
        "maker-del-1",
        &crate::plan::signals::WorkerSignal::MarkNoop {
            signal_id: uuid::Uuid::new_v4(),
            reason: "intentional no-code artifact result".into(),
        },
    )
    .await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());
    reconciler.tick_once().await.expect("mark-noop review tick");
    let review = next_review_request(&mut requests, "MarkNoop reviewer request").await;
    assert!(review.task.contains("MarkNoop"));

    let blocked_pm = crate::plan::test_util::MockPm::new().arc();
    let (blocked_issues, _, _) =
        seed_system_l3_awaiting_review_plan(&blocked_pm, "P-L3-REVIEW-BLOCK", &["T1"]).await;
    add_review_signal(
        blocked_pm.as_ref(),
        &blocked_issues[0],
        "maker-del-1",
        &crate::plan::signals::WorkerSignal::ScopeDrift {
            signal_id: uuid::Uuid::new_v4(),
            severity: 0.9,
            reason: "unresolved scope drift".into(),
            estimated_subtasks: Some(2),
        },
    )
    .await;
    let (blocked, mut blocked_requests) = system_l3_review_reconciler(blocked_pm);
    blocked.tick_once().await.expect("blocking signal tick");
    assert!(blocked_requests.try_recv().is_err());
}

#[tokio::test]
async fn system_l3_review_processed_signal_label_does_not_block_history() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-PROCESSED", &["T1"]).await;
    let signal_id = uuid::Uuid::new_v4();
    add_review_signal(
        pm.as_ref(),
        &issues[0],
        "maker-del-1",
        &crate::plan::signals::WorkerSignal::ScopeDrift {
            signal_id,
            severity: 0.8,
            reason: "historical drift already handled".into(),
            estimated_subtasks: Some(1),
        },
    )
    .await;
    add_mock_issue_label(
        &pm,
        &issues[0],
        crate::plan::labels::signal_processed_label(&signal_id),
    )
    .await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm);

    reconciler.tick_once().await.expect("processed signal tick");

    next_review_request(&mut requests, "processed history reviewer").await;
}

#[tokio::test]
async fn system_l3_review_does_not_reuse_mark_noop_from_an_older_maker_attempt() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-OLD-NOOP", &["T1"]).await;
    add_review_signal(
        pm.as_ref(),
        &issues[0],
        "maker-del-old",
        &crate::plan::signals::WorkerSignal::MarkNoop {
            signal_id: uuid::Uuid::new_v4(),
            reason: "old maker supplied this no-op justification".into(),
        },
    )
    .await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm);

    reconciler.tick_once().await.expect("review tick");

    let review = next_review_request(&mut requests, "current maker reviewer").await;
    assert!(
        !review
            .task
            .contains("old maker supplied this no-op justification"),
        "a reviewer must not receive MarkNoop evidence from another maker attempt"
    );
}

#[tokio::test]
async fn system_l3_review_blocks_unattributed_legacy_mark_noop() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-LEGACY-NOOP", &["T1"]).await;
    let signal = crate::plan::signals::WorkerSignal::MarkNoop {
        signal_id: uuid::Uuid::new_v4(),
        reason: "legacy no-op without authenticated maker attribution".into(),
    };
    crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        pro_feature_gate().as_ref(),
    )
    .expect("pro gate");
    pm.advanced()
        .expect("beads advanced")
        .add_comment(&issues[0], &crate::plan::signals::encode_comment(&signal))
        .await
        .expect("add legacy signal comment");
    add_mock_issue_label(
        &pm,
        &issues[0],
        crate::plan::labels::signal_kind(signal.kind_label()),
    )
    .await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm);

    reconciler.tick_once().await.expect("legacy signal tick");

    assert!(
        requests.try_recv().is_err(),
        "an unattributed legacy MarkNoop must fail closed"
    );
}

#[tokio::test]
async fn system_l3_review_blocks_signal_label_without_a_durable_fact() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-ORPHAN-LABEL", &["T1"]).await;
    add_mock_issue_label(&pm, &issues[0], crate::plan::labels::signal_kind("risk")).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm);

    reconciler
        .tick_once()
        .await
        .expect("orphan signal label tick");

    assert!(
        requests.try_recv().is_err(),
        "a signal label without a durable signal fact must fail closed"
    );
}

#[tokio::test]
async fn system_l3_review_approval_releases_dependent_and_request_changes_reuses_branch() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, maker_branch) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-APPROVE", &["T1", "T2"]).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());
    reconciler.tick_once().await.expect("dispatch reviewer");
    let review = next_review_request(&mut requests, "approval review request").await;
    submit_test_review_verdict(pm.as_ref(), &issues[0], review.id.as_str(), "approve").await;
    drop(review);

    reconciler.tick_once().await.expect("consume approval");

    let dependent = next_review_request(&mut requests, "dependent maker dispatch").await;
    assert_eq!(dependent.issue_id.as_deref(), Some(issues[1].as_str()));
    assert!(pm.issue(&issues[0]).await.status == "closed");

    let retry_pm = crate::plan::test_util::MockPm::new().arc();
    let (retry_issues, _, _) =
        seed_system_l3_awaiting_review_plan(&retry_pm, "P-L3-REVIEW-CHANGES", &["T1"]).await;
    let (retry_reconciler, mut retry_requests) = system_l3_review_reconciler(retry_pm.clone());
    retry_reconciler
        .tick_once()
        .await
        .expect("dispatch reviewer");
    let retry_review = next_review_request(&mut retry_requests, "changes review request").await;
    submit_test_review_verdict(
        retry_pm.as_ref(),
        &retry_issues[0],
        retry_review.id.as_str(),
        "request_changes",
    )
    .await;
    drop(retry_review);

    retry_reconciler
        .tick_once()
        .await
        .expect("consume request changes");

    let maker_retry = next_review_request(&mut retry_requests, "maker retry dispatch").await;
    assert_eq!(
        maker_retry.issue_id.as_deref(),
        Some(retry_issues[0].as_str())
    );
    assert_eq!(
        maker_retry.prior_branch_for_reuse.as_deref(),
        Some(maker_branch.as_str())
    );
}

#[tokio::test]
async fn system_l3_review_completion_without_verdict_never_approves() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-NO-VERDICT", &["T1"]).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());
    reconciler.tick_once().await.expect("dispatch reviewer");
    let review = next_review_request(&mut requests, "no-verdict review request").await;
    review
        .respond_to
        .send(spur_acp::DelegationResult {
            resolved_config: None,
            status: spur_acp::DelegationStatus::Success,
            diff: None,
            diff_summary: None,
            summary: Some("reviewer exited without verdict".into()),
            estimated_cost_usd: 0.0,
            worker_branch: Some("spur/worker/throwaway-review".into()),
            artifact: None,
        })
        .expect("return reviewer result");
    tokio::task::yield_now().await;

    reconciler.tick_once().await.expect("post-result tick");

    let audits = crate::plan::projector::collect_sorted_audits_for_issue(
        &issues[0],
        pm.comments(&issues[0]).await,
    )
    .expect("target audits");
    assert!(!audits.iter().any(|audit| matches!(
        audit,
        crate::plan::audit_sentinel::AuditSentinelKind::Approval { .. }
    )));
    assert_eq!(pm.issue(&issues[0]).await.status, "open");
}

#[tokio::test]
async fn system_l3_review_sweeps_reopened_companion_after_target_advanced() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-CLEANUP", &["T1"]).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());
    reconciler.tick_once().await.expect("dispatch reviewer");
    let review = next_review_request(&mut requests, "cleanup reviewer").await;
    let companion_id = review.issue_id.clone().expect("review companion");
    submit_test_review_verdict(pm.as_ref(), &issues[0], review.id.as_str(), "approve").await;

    reconciler.tick_once().await.expect("consume verdict");
    assert_eq!(pm.issue(&issues[0]).await.status, pm.closed_status());
    pm.update_issue(
        &companion_id,
        spur_pm::IssueUpdate {
            status: Some("open".into()),
            ..Default::default()
        },
    )
    .await
    .expect("simulate late generic completion reopening companion");

    reconciler.tick_once().await.expect("cleanup sweep");

    assert_eq!(
        pm.issue(&companion_id).await.status,
        pm.closed_status(),
        "an open companion with a durable verdict must be swept after target advancement"
    );
    drop(review);
}

#[tokio::test]
async fn system_l3_review_retries_companion_cleanup_after_close_failure() {
    let inner = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&inner, "P-L3-REVIEW-CLOSE-FAULT", &["T1"]).await;
    let pm = Arc::new(FailFirstCompanionClosePm {
        inner: Arc::clone(&inner),
        fail_close: std::sync::atomic::AtomicBool::new(true),
    });
    let (reconciler, mut requests) = system_l3_review_reconciler(pm);
    reconciler.tick_once().await.expect("dispatch reviewer");
    let review = next_review_request(&mut requests, "faulted cleanup reviewer").await;
    let companion_id = review.issue_id.clone().expect("review companion");
    submit_test_review_verdict(inner.as_ref(), &issues[0], review.id.as_str(), "approve").await;

    let error = reconciler
        .tick_once()
        .await
        .expect_err("first companion close is injected to fail");
    assert!(error
        .to_string()
        .contains("injected review companion close failure"));
    assert_eq!(inner.issue(&issues[0]).await.status, inner.closed_status());
    assert_eq!(inner.issue(&companion_id).await.status, "open");

    reconciler.tick_once().await.expect("retry cleanup sweep");

    assert_eq!(
        inner.issue(&companion_id).await.status,
        inner.closed_status(),
        "cleanup must be recoverable after target advancement"
    );
    drop(review);
}

async fn expire_review_lease(pm: &crate::plan::test_util::MockPm, review_issue_id: &str) {
    let issue = pm.issue(review_issue_id).await;
    let old = issue
        .labels
        .iter()
        .filter(|label| crate::plan::labels::parse_lease_expires_at(label).is_some())
        .cloned()
        .collect();
    pm.update_issue(
        review_issue_id,
        spur_pm::IssueUpdate {
            add_labels: vec![crate::plan::labels::lease_expires_at(0)],
            remove_labels: old,
            ..Default::default()
        },
    )
    .await
    .expect("expire review lease");
}

#[tokio::test]
async fn system_l3_review_three_expired_attempts_park_instead_of_approving() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let (issues, _, _) =
        seed_system_l3_awaiting_review_plan(&pm, "P-L3-REVIEW-FAILURES", &["T1"]).await;
    let (reconciler, mut requests) = system_l3_review_reconciler(pm.clone());
    reconciler.tick_once().await.expect("first review dispatch");
    let first = next_review_request(&mut requests, "first reviewer").await;
    let review_issue_id = first.issue_id.clone().expect("review issue id");
    drop(first);
    expire_review_lease(pm.as_ref(), &review_issue_id).await;

    reconciler
        .tick_once()
        .await
        .expect("second review dispatch");
    let second = next_review_request(&mut requests, "second reviewer").await;
    assert_eq!(second.issue_id.as_deref(), Some(review_issue_id.as_str()));
    drop(second);
    expire_review_lease(pm.as_ref(), &review_issue_id).await;

    reconciler.tick_once().await.expect("third review dispatch");
    let third = next_review_request(&mut requests, "third reviewer").await;
    assert_eq!(third.issue_id.as_deref(), Some(review_issue_id.as_str()));
    drop(third);
    expire_review_lease(pm.as_ref(), &review_issue_id).await;

    reconciler.tick_once().await.expect("park exhausted review");

    assert!(requests.try_recv().is_err());
    let target = pm.issue(&issues[0]).await;
    assert!(target
        .labels
        .contains(&crate::plan::labels::signal_kind("review-failed")));
    assert_eq!(target.status, "open");
}

#[test]
fn loop_sweep_scopes_select_only_their_autonomy_levels() {
    use crate::plan::loops::spec::AutonomyLevel;
    use crate::plan::loops::LoopSweepScope;

    assert!(LoopSweepScope::L3Only.allows(AutonomyLevel::L3));
    assert!(!LoopSweepScope::L3Only.allows(AutonomyLevel::L1));
    assert!(!LoopSweepScope::BrainArmedOnly.allows(AutonomyLevel::L3));
    assert!(LoopSweepScope::BrainArmedOnly.allows(AutonomyLevel::L2));
}

#[tokio::test]
async fn system_l3_reconciler_ignores_brain_owned_non_l3_plans() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-owned-l2".into()));
    let ready_task_id =
        seed_mock_ready_task_plan(&pm, "P-BRAIN-READY", "T1", &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-BRAIN-READY",
        format!("{}l2", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;

    let terminal_task_id =
        seed_mock_ready_task_plan(&pm, "P-BRAIN-TERMINAL", "T1", &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-BRAIN-TERMINAL",
        format!("{}l2", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    close_mock_task_with_completion_cost(&pm, &terminal_task_id, 0).await;

    let system_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(
        crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
    ));
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig {
            loop_sweep_scope: crate::plan::loops::LoopSweepScope::L3Only,
            plan_scope: PlanScope::SystemL3Only,
            ..Default::default()
        },
        pm.clone(),
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, system_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    let ready = reconciler.observe_ready_summaries().await.unwrap();
    assert!(
        ready
            .iter()
            .all(|candidate| candidate.summary.id != ready_task_id),
        "system reconciler discovered a brain-owned ready task"
    );
    reconciler.tick_once().await.unwrap();

    let issues = pm.issues().await;
    let terminal_epic = issues
        .iter()
        .find(|issue| {
            issue.issue_type.as_deref() == Some("epic")
                && issue
                    .labels
                    .contains(&crate::plan::labels::plan_id("P-BRAIN-TERMINAL"))
        })
        .expect("terminal brain-owned epic");
    assert_eq!(terminal_epic.status, "open");
}

async fn seed_historical_l3_plan(
    pm: &Arc<crate::plan::test_util::MockPm>,
    plan_id: &str,
    owner: &str,
) -> (String, String) {
    let owner_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(owner.into()));
    let task_id = seed_mock_ready_task_plan(pm, plan_id, "T1", &owner_id).await;
    add_mock_epic_label(
        pm,
        plan_id,
        format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    let epic_id = pm
        .issues()
        .await
        .into_iter()
        .find(|issue| {
            issue.issue_type.as_deref() == Some("epic")
                && issue
                    .labels
                    .contains(&crate::plan::labels::plan_id(plan_id))
        })
        .expect("historical L3 epic")
        .id;
    (epic_id, task_id)
}

#[tokio::test]
async fn system_l3_runtime_never_adopts_legacy_owner() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let scenarios = [
        ("NO-LEASE", None),
        ("EXPIRED-LEASE", Some(500)),
        ("LIVE-LEASE", Some(20_000)),
    ];
    let mut expected = Vec::new();
    for (suffix, lease) in scenarios {
        let plan_id = format!("P-L3-{suffix}");
        let prior_owner = format!("historical-{suffix}").to_ascii_lowercase();
        let (epic_id, _) = seed_historical_l3_plan(&pm, &plan_id, &prior_owner).await;
        let mut add_labels = vec![crate::plan::labels::plan_owner_token(&format!(
            "token-{suffix}"
        ))];
        if let Some(expiry) = lease {
            add_labels.push(crate::plan::labels::plan_owner_lease_expires_at(expiry));
        }
        pm.update_issue(
            &epic_id,
            spur_pm::IssueUpdate {
                add_labels,
                ..Default::default()
            },
        )
        .await
        .expect("seed legacy fencing labels");
        let comment_bodies = pm
            .comments(&epic_id)
            .await
            .into_iter()
            .map(|comment| comment.body)
            .collect::<Vec<_>>();
        expected.push((
            epic_id.clone(),
            pm.issue(&epic_id).await.labels,
            comment_bodies,
        ));
    }

    for now in [1_000, 10_000] {
        system_l3_reconciler(pm.clone(), now)
            .tick_once()
            .await
            .expect("system L3 tick");
    }

    for (epic_id, expected_labels, expected_comments) in expected {
        let epic = pm.issue(&epic_id).await;
        assert_eq!(
            epic.labels, expected_labels,
            "system runtime must never mutate a foreign brain-owned L3 generation"
        );
        assert_eq!(
            pm.comments(&epic_id)
                .await
                .into_iter()
                .map(|comment| comment.body)
                .collect::<Vec<_>>(),
            expected_comments,
            "ignoring a legacy generation must not fabricate an ownership audit",
        );
    }
}

fn system_l3_reconciler(pm: Arc<crate::plan::test_util::MockPm>, now: u64) -> Reconciler {
    let system_id = spur_acp::BrainSessionId::new(spur_acp::SessionId(
        crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into(),
    ));
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel(4);
    let mut reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig {
            loop_sweep_scope: crate::plan::loops::LoopSweepScope::L3Only,
            plan_scope: PlanScope::SystemL3Only,
            ..Default::default()
        },
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, system_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );
    reconciler.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(now),
    }));
    reconciler
}

fn l3_recovery_template() -> serde_json::Value {
    serde_json::json!({
        "epic_title": "Recovered L3 generation",
        "tasks": [{
            "task_id": "T1",
            "agent": "codex",
            "task": "Run the recovered generation"
        }]
    })
}

async fn seed_mock_l3_generation_epic(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_id: &str,
    generation: u32,
) -> String {
    let owner = spur_acp::SessionId(crate::plan::loops::LOOP_RUNTIME_OWNER_ID.into());
    let plan_id = format!("P-{loop_id}-{generation}");
    let epic_id = seed_mock_complete_epic_for_owner(pm, &plan_id, &owner).await;
    for label in [
        crate::plan::labels::loop_id_label(loop_id),
        crate::plan::labels::loop_generation_label(generation),
        format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX),
    ] {
        add_mock_epic_label(pm, &plan_id, label).await;
    }
    epic_id
}

async fn generation_epics_for(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_id: &str,
    generation: u32,
) -> Vec<spur_pm::Issue> {
    pm.issues()
        .await
        .into_iter()
        .filter(|issue| {
            issue.issue_type.as_deref() == Some("epic")
                && issue
                    .labels
                    .contains(&crate::plan::labels::loop_id_label(loop_id))
                && issue
                    .labels
                    .contains(&crate::plan::labels::loop_generation_label(generation))
        })
        .collect()
}

async fn loop_has_future_next_run(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_issue_id: &str,
    now: i64,
) -> bool {
    pm.issues()
        .await
        .into_iter()
        .find(|issue| issue.id == loop_issue_id)
        .is_some_and(|issue| {
            issue
                .labels
                .iter()
                .filter_map(|label| crate::plan::labels::parse_loop_next_run(label))
                .any(|next_run| next_run > now)
        })
}

async fn loop_has_arming_label(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_issue_id: &str,
) -> bool {
    pm.issues()
        .await
        .into_iter()
        .find(|issue| issue.id == loop_issue_id)
        .is_some_and(|issue| {
            issue
                .labels
                .iter()
                .any(|label| crate::plan::labels::parse_loop_arming(label).is_some())
        })
}

async fn loop_run_outcomes(
    pm: &Arc<crate::plan::test_util::MockPm>,
    loop_issue_id: &str,
) -> Vec<String> {
    pm.comments(loop_issue_id)
        .await
        .into_iter()
        .filter_map(|comment| crate::plan::audit_sentinel::parse_comment(&comment.body))
        .filter_map(Result::ok)
        .filter_map(|audit| match audit {
            crate::plan::audit_sentinel::AuditSentinelKind::LoopRun { outcome, .. } => {
                Some(outcome)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn l3_claim_with_exact_generation_repairs_without_false_overlap() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let loop_id = "claimed-exact";
    let loop_issue_id = create_mock_loop_issue(
        &pm,
        loop_id,
        crate::plan::loops::spec::AutonomyLevel::L3,
        l3_recovery_template(),
        None,
    )
    .await;
    pm.update_issue(
        &loop_issue_id,
        spur_pm::IssueUpdate {
            add_labels: vec![crate::plan::labels::loop_arming_label(1)],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    seed_mock_l3_generation_epic(&pm, loop_id, 1).await;
    let reconciler = system_l3_reconciler(pm.clone(), 10);

    reconciler.tick_once().await.unwrap();

    assert_eq!(generation_epics_for(&pm, loop_id, 1).await.len(), 1);
    assert!(loop_has_future_next_run(&pm, &loop_issue_id, 10).await);
    assert!(!loop_has_arming_label(&pm, &loop_issue_id).await);
    assert!(!loop_run_outcomes(&pm, &loop_issue_id)
        .await
        .contains(&"skipped_overlap".to_string()));
}

#[tokio::test]
async fn l3_claim_without_generation_creates_exact_generation_once() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let loop_id = "claimed-absent";
    let loop_issue_id = create_mock_loop_issue(
        &pm,
        loop_id,
        crate::plan::loops::spec::AutonomyLevel::L3,
        l3_recovery_template(),
        None,
    )
    .await;
    pm.update_issue(
        &loop_issue_id,
        spur_pm::IssueUpdate {
            add_labels: vec![crate::plan::labels::loop_arming_label(1)],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let reconciler = system_l3_reconciler(pm.clone(), 10);

    reconciler.tick_once().await.unwrap();
    reconciler.tick_once().await.unwrap();

    assert_eq!(generation_epics_for(&pm, loop_id, 1).await.len(), 1);
    assert!(loop_has_future_next_run(&pm, &loop_issue_id, 10).await);
    assert!(!loop_has_arming_label(&pm, &loop_issue_id).await);
    assert!(!loop_run_outcomes(&pm, &loop_issue_id)
        .await
        .contains(&"skipped_overlap".to_string()));
}

#[tokio::test]
async fn l3_without_claim_retains_real_older_generation_overlap() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let loop_id = "older-live";
    let loop_issue_id = create_mock_loop_issue(
        &pm,
        loop_id,
        crate::plan::loops::spec::AutonomyLevel::L3,
        l3_recovery_template(),
        None,
    )
    .await;
    seed_mock_l3_generation_epic(&pm, loop_id, 1).await;
    let reconciler = system_l3_reconciler(pm.clone(), 10);

    reconciler.tick_once().await.unwrap();

    assert!(generation_epics_for(&pm, loop_id, 2).await.is_empty());
    assert!(loop_run_outcomes(&pm, &loop_issue_id)
        .await
        .contains(&"skipped_overlap".to_string()));
}

#[tokio::test(start_paused = true)]
async fn l1_and_l2_still_push_continuations() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let template = serde_json::json!({
        "tasks": [
            {
                "task_id": "Triage",
                "agent": "codex",
                "task": "Brain should author this generation"
            }
        ]
    });
    create_mock_loop_issue(
        &pm,
        "loop-l1",
        crate::plan::loops::spec::AutonomyLevel::L1,
        template.clone(),
        None,
    )
    .await;
    create_mock_loop_issue(
        &pm,
        "loop-l2",
        crate::plan::loops::spec::AutonomyLevel::L2,
        template,
        None,
    )
    .await;
    let brain_session_id = spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-l1-l2".into()));
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let (dispatch, continuations) =
        test_dispatch_ctx_with_recording(delegation_tx, brain_session_id, None);
    let mut reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone(),
        Arc::new(Notify::new()),
        Some(dispatch.into_dispatch()),
        None,
        pro_feature_gate(),
    );
    reconciler.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(20),
    }));

    reconciler.tick_once().await.expect("tick once");

    let continuations = continuations.lock().expect("continuations lock");
    assert_eq!(continuations.len(), 2);
    assert!(continuations.iter().all(|continuation| {
        continuation.source == spur_acp::domain::ContinuationSource::LoopDue
    }));
    let loop_generation_epics = pm
        .issues()
        .await
        .into_iter()
        .filter(|issue| {
            issue.issue_type.as_deref() == Some("epic")
                && issue
                    .labels
                    .iter()
                    .any(|label| crate::plan::labels::parse_loop_generation(label).is_some())
        })
        .count();
    assert_eq!(loop_generation_epics, 0);
}

#[tokio::test(start_paused = true)]
async fn invalid_template_auto_pauses_loop_with_escalation_record() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let loop_id = "loop-invalid-template";
    let loop_issue_id = create_mock_loop_issue(
        &pm,
        loop_id,
        crate::plan::loops::spec::AutonomyLevel::L3,
        serde_json::json!({ "tasks": "not an array" }),
        None,
    )
    .await;
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-invalid-template".into()));
    let event_sink = Arc::new(RecordingEventSink::default());
    let (delegation_tx, _delegation_rx) = tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let (dispatch, continuations) =
        test_dispatch_ctx_with_recording(delegation_tx, brain_session_id, Some(event_sink.clone()));
    let mut reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone(),
        Arc::new(Notify::new()),
        Some(dispatch.into_dispatch()),
        None,
        pro_feature_gate(),
    );
    reconciler.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(30),
    }));

    let did_work = reconciler.tick_once().await.expect("tick once");

    assert!(did_work, "invalid template pause is durable scheduler work");
    let loop_issue = pm.issue(&loop_issue_id).await;
    assert!(loop_issue
        .labels
        .contains(&crate::plan::labels::LOOP_PAUSED.to_string()));
    assert!(
        !loop_issue
            .labels
            .iter()
            .any(|label| crate::plan::labels::parse_loop_next_run(label).is_some()),
        "paused invalid template loop should not be rearmed: {:?}",
        loop_issue.labels
    );
    let continuations = continuations.lock().expect("continuations lock");
    assert_eq!(continuations.len(), 1);
    assert_eq!(
        continuations[0].source,
        spur_acp::domain::ContinuationSource::LoopEscalation
    );
    let comments = pm.comments(&loop_issue_id).await;
    let audits = comments
        .iter()
        .filter_map(|comment| crate::plan::audit_sentinel::parse_comment(&comment.body))
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert!(
        audits.iter().any(|audit| matches!(
            audit,
            crate::plan::audit_sentinel::AuditSentinelKind::LoopRun {
                loop_id: found_loop_id,
                generation: 1,
                outcome,
                escalations: 1,
                ..
            } if found_loop_id == loop_id && outcome == "invalid_template"
        )),
        "expected invalid-template LoopRun escalation record, got {audits:?}"
    );
    let events = event_sink.events.lock().expect("events lock");
    assert!(
        events.iter().any(|event| matches!(
            event,
            spur_acp::SpurEventBody::LoopPaused {
                loop_id: found_loop_id,
                by,
            } if found_loop_id == loop_id && by == "auto_paused"
        )),
        "expected LoopPaused auto_paused event, got {events:?}"
    );
}

fn drain_delegation_requests(
    delegation_rx: &mut tokio::sync::mpsc::Receiver<crate::DelegationRequest>,
) -> Vec<crate::DelegationRequest> {
    let mut requests = Vec::new();
    loop {
        match delegation_rx.try_recv() {
            Ok(request) => requests.push(request),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    requests
}

#[tokio::test(start_paused = true)]
async fn budget_exhausted_plan_suppresses_dispatch() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-budget".into()));
    let issue_ids =
        seed_mock_ready_tasks_plan(&pm, "P-BUDGET", &["Spent", "Ready"], &brain_session_id).await;
    close_mock_task_with_completion_cost(&pm, &issue_ids[0], 1_200_000).await;
    add_mock_epic_label(
        &pm,
        "P-BUDGET",
        format!("{}1000000", crate::plan::labels::LOOP_BUDGET_MICROS_PREFIX),
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    assert!(
        delegation_rx.try_recv().is_err(),
        "over-budget plan must not dispatch a worker"
    );
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let recent = outcomes.recent_outcomes("P-BUDGET");
    assert!(
        recent.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Skipped {
                task_id,
                reason: SkipReason::BudgetExhausted {
                    spent_micros: 1_200_000,
                    cap_micros: 1_000_000,
                },
                ..
            } if task_id == "Ready"
        )),
        "expected budget-exhausted skip, got {recent:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn epic_with_loop_paused_label_suppresses_dispatch() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-loop-paused".into()));
    seed_mock_ready_task_plan(&pm, "P-LOOP-PAUSED", "T1", &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-LOOP-PAUSED",
        crate::plan::labels::LOOP_PAUSED.to_string(),
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    assert!(
        delegation_rx.try_recv().is_err(),
        "loop-paused plan must not dispatch a worker"
    );
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let recent = outcomes.recent_outcomes("P-LOOP-PAUSED");
    assert!(
        recent.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Skipped {
                task_id,
                reason: SkipReason::LoopsPaused { scope },
                ..
            } if task_id == "T1" && scope == "loop"
        )),
        "expected loop-scoped pause skip, got {recent:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn epic_with_pause_all_loops_label_suppresses_dispatch() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-pause-all".into()));
    seed_mock_ready_task_plan(&pm, "P-PAUSE-ALL", "T1", &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-PAUSE-ALL",
        crate::plan::labels::PAUSE_ALL_LOOPS.to_string(),
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    assert!(
        delegation_rx.try_recv().is_err(),
        "global pause label must not dispatch a worker"
    );
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let recent = outcomes.recent_outcomes("P-PAUSE-ALL");
    assert!(
        recent.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Skipped {
                task_id,
                reason: SkipReason::LoopsPaused { scope },
                ..
            } if task_id == "T1" && scope == "global"
        )),
        "expected global pause skip, got {recent:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn l1_autonomy_suppresses_non_triage_tasks() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-l1-report".into()));
    let issue_ids =
        seed_mock_ready_tasks_plan(&pm, "P-L1", &["Triage", "Action"], &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-L1",
        format!("{}l1", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    add_mock_issue_label(
        &pm,
        &issue_ids[0],
        crate::plan::labels::LOOP_TRIAGE_TASK.to_string(),
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(2);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    let requests = drain_delegation_requests(&mut delegation_rx);
    assert_eq!(requests.len(), 1, "L1 should dispatch only triage");
    assert_eq!(requests[0].issue_id.as_deref(), Some(issue_ids[0].as_str()));
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let recent = outcomes.recent_outcomes("P-L1");
    assert!(
        recent.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Skipped {
                task_id,
                reason: SkipReason::ReportOnly,
                ..
            } if task_id == "Action"
        )),
        "expected report-only skip for non-triage task, got {recent:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn l2_autonomy_dispatches_all_ready_tasks() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-l2-assisted".into()));
    let issue_ids =
        seed_mock_ready_tasks_plan(&pm, "P-L2", &["Triage", "Action"], &brain_session_id).await;
    add_mock_epic_label(
        &pm,
        "P-L2",
        format!("{}l2", crate::plan::labels::AUTONOMY_PREFIX),
    )
    .await;
    add_mock_issue_label(
        &pm,
        &issue_ids[0],
        crate::plan::labels::LOOP_TRIAGE_TASK.to_string(),
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(2);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    let requests = drain_delegation_requests(&mut delegation_rx);
    let request_issue_ids = requests
        .iter()
        .filter_map(|request| request.issue_id.as_deref())
        .collect::<HashSet<_>>();
    assert_eq!(
        request_issue_ids,
        HashSet::from([issue_ids[0].as_str(), issue_ids[1].as_str()])
    );
}

#[tokio::test]
async fn completion_blocks_observed_write_collision_between_parallel_siblings() {
    let repo = tempfile::tempdir().expect("temp repo");
    let base_oid = seed_git_repo(repo.path());
    let first_branch = create_worker_branch(repo.path(), "spur/worker-observed-first", "shared.rs");
    let second_branch =
        create_worker_branch(repo.path(), "spur/worker-observed-second", "shared.rs");
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-observed-writes".into()));
    let issue_ids = seed_mock_ready_tasks_plan(
        &pm,
        "P-OBSERVED-WRITES",
        &["first", "second"],
        &brain_session_id,
    )
    .await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(2);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig {
            repo_root: repo.path().to_path_buf(),
            predispatch_preview: PreviewStrategy::AlwaysClean,
            ..Default::default()
        },
        pm.clone(),
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler
        .tick_once()
        .await
        .expect("dispatch both siblings");
    let mut requests = vec![
        delegation_rx.recv().await.expect("first request"),
        delegation_rx.recv().await.expect("second request"),
    ];
    let first_index = requests
        .iter()
        .position(|request| request.issue_id.as_deref() == Some(issue_ids[0].as_str()))
        .expect("first task request");
    let first_request = requests.swap_remove(first_index);
    let second_request = requests.pop().expect("second task request");

    let complete = |request: crate::DelegationRequest, branch: String| {
        if let Some(base_tx) = &request.dispatched_base_oid_tx {
            base_tx
                .send(Some(base_oid.clone()))
                .expect("publish dispatched base oid");
        }
        request
            .respond_to
            .send(spur_acp::DelegationResult {
                resolved_config: None,
                status: spur_acp::DelegationStatus::Success,
                diff: Some("diff --git a/shared.rs b/shared.rs".into()),
                diff_summary: Some(spur_acp::DiffSummary {
                    files_changed: 1,
                    insertions: 1,
                    deletions: 0,
                    files: vec![std::path::PathBuf::from("shared.rs")],
                }),
                summary: Some("updated shared runtime".into()),
                estimated_cost_usd: 0.0,
                worker_branch: Some(branch),
                artifact: None,
            })
            .expect("return worker result");
    };

    complete(first_request, first_branch);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if pm
                .issue(&issue_ids[0])
                .await
                .labels
                .contains(&crate::plan::labels::READY_FOR_REVIEW.to_string())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first sibling reaches review");

    complete(second_request, second_branch);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let labels = pm.issue(&issue_ids[1]).await.labels;
            if labels.contains(&crate::plan::labels::SIGNAL_LABEL_INTEGRATION_CONFLICT.to_string())
                || labels.contains(&crate::plan::labels::READY_FOR_REVIEW.to_string())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second sibling completion is persisted");

    let second = pm.issue(&issue_ids[1]).await;
    assert!(
        second
            .labels
            .contains(&crate::plan::labels::SIGNAL_LABEL_INTEGRATION_CONFLICT.to_string()),
        "observed overlapping writes must surface a structured integration-conflict signal"
    );
    assert!(
        !second
            .labels
            .contains(&crate::plan::labels::READY_FOR_REVIEW.to_string()),
        "collision must be blocked before AwaitingReview"
    );
    let signals = pm
        .comments(&issue_ids[1])
        .await
        .into_iter()
        .filter_map(|comment| {
            comment
                .body
                .trim_start()
                .strip_prefix(crate::plan::signals::SENTINEL_PREFIX)
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json.trim()).ok())
        })
        .collect::<Vec<_>>();
    assert!(
        signals.iter().any(|signal| {
            signal
                .get("kind")
                .is_some_and(|kind| kind == "integration_conflict")
                && signal
                    .get("dep_task_id")
                    .is_some_and(|task| task == "first")
                && signal
                    .get("files")
                    .is_some_and(|files| files == &serde_json::json!(["shared.rs"]))
        }),
        "expected deterministic observed-write conflict payload, got {signals:?}"
    );
    let projected = crate::plan::projector::project_plan_from_beads(
        pm.as_ref(),
        "P-OBSERVED-WRITES",
        &pro_feature_gate(),
    )
    .await
    .expect("project observed-write conflict");
    let second_projected = projected
        .tasks
        .iter()
        .find(|entry| entry.spec.task_id == "second")
        .expect("projected second sibling");
    assert!(matches!(
        &second_projected.status,
        crate::plan::PlanTaskStatus::BlockedOnSetupConflict { dep_task_id, files }
            if dep_task_id == "first" && files == &["shared.rs".to_string()]
    ));
}

#[tokio::test(start_paused = true)]
async fn plan_task_profile_round_trips_to_dispatch_and_retry() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-profile-plan".into()));
    let issue_id = seed_mock_ready_task_plan(&pm, "P-PROFILE", "T1", &brain_session_id).await;
    let adv =
        crate::plan::PmLike::advanced(pm.as_ref()).expect("mock pm should expose beads advanced");
    crate::plan::emit_task_spec_audit(
        adv,
        &issue_id,
        "T1",
        "codex",
        Some("code-reviewer"),
        None,
        None,
        None,
        None,
        &[],
        None,
    )
    .await
    .expect("profile task spec audit");
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(2);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone(),
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("initial tick");
    let first = delegation_rx.recv().await.expect("initial dispatch");
    let first_delegation_id = first.id.as_str().to_string();
    assert_eq!(first.profile.as_deref(), Some("code-reviewer"));

    adv.add_comment(
        &issue_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::Completion {
                delegation_id: first_delegation_id.clone(),
                completion_state: crate::plan::audit_sentinel::CompletionState::Failed,
                superseded: false,
                worker_branch: None,
                result_summary: Some("failed before retry".to_string()),
                artifact_uri: None,
                dispatched_base_oid: None,
                estimated_cost_micros: None,
            },
        ),
    )
    .await
    .expect("failed completion audit");
    crate::plan::clear_dispatch_intent(pm.as_ref(), &issue_id, &first_delegation_id)
        .await
        .expect("clear first dispatch labels");
    adv.add_comment(
        &issue_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::RetryRequested {
                delegation_id: first_delegation_id,
                attempt: 1,
                error: "brain retry".to_string(),
                worker_branch: None,
                amended_prompt_summary: None,
            },
        ),
    )
    .await
    .expect("retry requested audit");

    reconciler.tick_once().await.expect("retry tick");
    let retry = delegation_rx.recv().await.expect("retry dispatch");
    assert_eq!(retry.profile.as_deref(), Some("code-reviewer"));
}

#[tokio::test(start_paused = true)]
async fn plan_task_skills_round_trips_to_dispatch() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-skills-plan".into()));
    let issue_id = seed_mock_ready_task_plan(&pm, "P-SKILLS", "T1", &brain_session_id).await;
    let adv =
        crate::plan::PmLike::advanced(pm.as_ref()).expect("mock pm should expose beads advanced");
    let skills = vec!["clean-a".to_string()];
    crate::plan::emit_task_spec_audit(
        adv,
        &issue_id,
        "T1",
        "codex",
        None,
        Some(&skills),
        None,
        None,
        None,
        &[],
        None,
    )
    .await
    .expect("skills task spec audit");
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");

    let request = delegation_rx.recv().await.expect("dispatch request");
    assert_eq!(request.skills.as_deref(), Some(skills.as_slice()));
}

#[tokio::test]
async fn global_reconciler_records_plan_no_ready_when_list_ready_empty_for_that_plan() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-global-ready".into()));
    seed_mock_complete_epic(&pm, "P1", &brain_session_id).await;
    seed_mock_ready_task_plan(&pm, "P2", "T2", &brain_session_id).await;
    let scripted_pm = Arc::new(ScriptedReadyPm {
        inner: Arc::clone(&pm),
        empty_plan_ids: HashSet::from(["P1".to_string()]),
    });
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        scripted_pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    let did_work = reconciler.tick_once().await.expect("tick once");
    let request = delegation_rx.recv().await.expect("dispatch request");
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let p1_outcomes = outcomes.recent_outcomes("P1");
    let p2_outcomes = outcomes.recent_outcomes("P2");

    assert!(did_work, "mixed global tick should dispatch P2");
    assert_eq!(request.issue_id.as_deref(), Some("bd-mock-3"));
    assert_eq!(
        p1_outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                DispatchOutcome::NoReadyTasks {
                    plan_id,
                    reason: NoReadyReason::NoMatchingRows,
                    ..
                } if plan_id == "P1"
            ))
            .count(),
        1,
        "P1 should record one per-plan NoReadyTasks outcome, got {p1_outcomes:?}"
    );
    assert!(
        p2_outcomes.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Dispatched { task_id, agent, .. }
                if task_id == "T2" && agent == "codex"
        )),
        "P2 should record a dispatched outcome, got {p2_outcomes:?}"
    );
}

#[tokio::test]
async fn global_reconciler_skips_other_brain_plan_without_emitting_no_ready() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-global-owner".into()));
    seed_mock_complete_epic_for_owner(
        &pm,
        "P1",
        &spur_acp::SessionId("other-brain-session".into()),
    )
    .await;
    seed_mock_ready_task_plan(&pm, "P2", "T2", &brain_session_id).await;
    let scripted_pm = Arc::new(ScriptedReadyPm {
        inner: Arc::clone(&pm),
        empty_plan_ids: HashSet::from(["P1".to_string()]),
    });
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        scripted_pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    let did_work = reconciler.tick_once().await.expect("tick once");
    let request = delegation_rx.recv().await.expect("dispatch request");
    let outcomes_store = reconciler.outcomes();
    let outcomes = outcomes_store.lock().await;
    let p1_outcomes = outcomes.recent_outcomes("P1");
    let p2_outcomes = outcomes.recent_outcomes("P2");
    let status = outcomes.reconciler_status();

    assert!(did_work, "mixed global tick should dispatch P2");
    assert_eq!(request.issue_id.as_deref(), Some("bd-mock-3"));
    assert_eq!(status.last_tick_plans_enumerated, 1);
    assert!(
        !p1_outcomes.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::NoReadyTasks {
                plan_id,
                reason: NoReadyReason::NoMatchingRows,
                ..
            } if plan_id == "P1"
        )),
        "P1 should not record cross-brain NoReadyTasks, got {p1_outcomes:?}"
    );
    assert!(
        p2_outcomes.iter().any(|outcome| matches!(
            outcome,
            DispatchOutcome::Dispatched { task_id, agent, .. }
                if task_id == "T2" && agent == "codex"
        )),
        "P2 should record a dispatched outcome, got {p2_outcomes:?}"
    );
}

#[tokio::test]
async fn global_reconciler_status_reports_plans_enumerated_and_dispatched_per_tick() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-global-status".into()));
    seed_mock_complete_epic(&pm, "P1", &brain_session_id).await;
    seed_mock_ready_task_plan(&pm, "P2", "T2", &brain_session_id).await;
    let scripted_pm = Arc::new(ScriptedReadyPm {
        inner: Arc::clone(&pm),
        empty_plan_ids: HashSet::from(["P1".to_string()]),
    });
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        scripted_pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");
    let _request = delegation_rx.recv().await.expect("dispatch request");
    let outcomes_store = reconciler.outcomes();
    let status = outcomes_store.lock().await.reconciler_status();

    assert_eq!(status.last_tick_plans_enumerated, 2);
    assert_eq!(status.last_tick_tasks_dispatched, 1);
}

#[tokio::test]
async fn global_reconciler_status_reports_tasks_dispatched_per_tick() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-global-task-count".into()));
    seed_mock_ready_tasks_plan(&pm, "P1", &["T1", "T2", "T3"], &brain_session_id).await;
    let scripted_pm = Arc::new(ScriptedReadyPm {
        inner: Arc::clone(&pm),
        empty_plan_ids: HashSet::new(),
    });
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(3);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        scripted_pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("tick once");
    let mut requests = Vec::new();
    for _ in 0..3 {
        requests.push(delegation_rx.recv().await.expect("dispatch request"));
    }
    let outcomes_store = reconciler.outcomes();
    let status = outcomes_store.lock().await.reconciler_status();

    assert_eq!(requests.len(), 3);
    assert_eq!(status.last_tick_plans_enumerated, 1);
    assert_eq!(status.last_tick_tasks_dispatched, 3);
}

#[tokio::test]
async fn last_tick_counters_reset_between_ticks() {
    let pm = crate::plan::test_util::MockPm::new().arc();
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-global-counter-reset".into()));
    seed_mock_ready_tasks_plan(&pm, "P1", &["T1", "T2", "T3"], &brain_session_id).await;
    let scripted_pm = Arc::new(ScriptedReadyPm {
        inner: Arc::clone(&pm),
        empty_plan_ids: HashSet::new(),
    });
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(3);
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        scripted_pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id).into_dispatch()),
        None,
        pro_feature_gate(),
    );

    reconciler.tick_once().await.expect("first tick");
    let mut requests = Vec::new();
    for _ in 0..3 {
        requests.push(delegation_rx.recv().await.expect("dispatch request"));
    }
    let outcomes_store = reconciler.outcomes();
    let first_status = outcomes_store.lock().await.reconciler_status();

    assert_eq!(requests.len(), 3);
    assert_eq!(first_status.last_tick_tasks_dispatched, 3);

    reconciler.tick_once().await.expect("second tick");
    let second_status = outcomes_store.lock().await.reconciler_status();

    assert_eq!(second_status.last_tick_tasks_dispatched, 0);
    assert_eq!(second_status.last_tick_plans_enumerated, 1);
}

async fn seed_ready_overlay_plan(
    repo: &std::path::Path,
    plan_id: &str,
    brain_session_id: &spur_acp::BrainSessionId,
) -> (Arc<spur_pm::PmService>, String, String) {
    let base_oid = seed_git_repo(repo);
    let x_worker_branch = create_worker_branch(repo, "spur/worker-X", "x.rs");
    let z_worker_branch = create_worker_branch(repo, "spur/worker-Z", "z.rs");
    let empty = spur_pm::test_workspace::TestBeadsWorkspace::init();
    let beads_dir = repo.join(".beads");
    std::fs::create_dir(&beads_dir).expect("create test .beads directory");
    empty.copy_db_to(&beads_dir);
    let pm = pm_for_beads_repo(repo).await;
    let feature_gate = pro_feature_gate();
    crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        feature_gate.as_ref(),
    )
    .expect("test feature gate should allow beads advanced");
    let adv = pm.advanced().expect("beads advanced");

    let epic_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "Predispatch Preview Plan".into(),
            description: Some("test plan".into()),
            issue_type: Some("epic".into()),
            labels: vec![
                crate::plan::labels::plan_id(plan_id),
                crate::plan::labels::PLAN_COMPLETE.to_string(),
                crate::plan::labels::plan_owner(&brain_session_id.as_session_id().0),
            ],
            ..Default::default()
        })
        .await
        .expect("create epic");
    let dep_issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "X: approved dep".into(),
            description: Some("approved dependency".into()),
            issue_type: Some("task".into()),
            labels: vec![
                crate::plan::labels::plan_id(plan_id),
                crate::plan::labels::plan_task_id("X"),
                crate::plan::labels::agent("codex"),
            ],
            parent: Some(epic_id.clone()),
            ..Default::default()
        })
        .await
        .expect("create dep task");
    let second_dep_issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "Z: approved dep".into(),
            description: Some("approved dependency".into()),
            issue_type: Some("task".into()),
            labels: vec![
                crate::plan::labels::plan_id(plan_id),
                crate::plan::labels::plan_task_id("Z"),
                crate::plan::labels::agent("codex"),
            ],
            parent: Some(epic_id.clone()),
            ..Default::default()
        })
        .await
        .expect("create second dep task");
    let ready_issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "Y: ready task".into(),
            description: Some("ready task".into()),
            issue_type: Some("task".into()),
            labels: vec![
                crate::plan::labels::plan_id(plan_id),
                crate::plan::labels::plan_task_id("Y"),
                crate::plan::labels::agent("codex"),
            ],
            parent: Some(epic_id.clone()),
            depends_on: vec![dep_issue_id.clone(), second_dep_issue_id.clone()],
            ..Default::default()
        })
        .await
        .expect("create ready task");

    adv.add_comment(
        &epic_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::PlanSubmit {
                plan_id: plan_id.to_string(),
                epic_issue_id: epic_id.clone(),
                task_ids: vec![
                    dep_issue_id.clone(),
                    second_dep_issue_id.clone(),
                    ready_issue_id.clone(),
                ],
                base_snapshot_branch: Some("main".to_string()),
                base_snapshot_oid: None,
                execution_mode: None,
                brain_session_id: Some(brain_session_id.as_session_id().0.clone()),
                explicit_base: None,
            },
        ),
    )
    .await
    .expect("plan submit audit");
    crate::plan::emit_task_spec_audit(
        adv,
        &dep_issue_id,
        "X",
        "codex",
        None,
        None,
        None,
        None,
        None,
        &["x.rs".to_string()],
        None,
    )
    .await
    .expect("dep task spec audit");
    crate::plan::emit_task_spec_audit(
        adv,
        &second_dep_issue_id,
        "Z",
        "codex",
        None,
        None,
        None,
        None,
        None,
        &["z.rs".to_string()],
        None,
    )
    .await
    .expect("second dep task spec audit");
    crate::plan::emit_task_spec_audit(
        adv,
        &ready_issue_id,
        "Y",
        "codex",
        None,
        None,
        None,
        None,
        None,
        &["y.rs".to_string()],
        None,
    )
    .await
    .expect("ready task spec audit");

    for (issue_id, task_id, worker_branch) in [
        (&dep_issue_id, "X", &x_worker_branch),
        (&second_dep_issue_id, "Z", &z_worker_branch),
    ] {
        let delegation_id = format!("del-{task_id}");
        crate::plan::emit_completion_audit(
            Some(pm.as_ref()),
            &Some(issue_id.to_string()),
            feature_gate.as_ref(),
            plan_id,
            &delegation_id,
            crate::plan::audit_sentinel::CompletionState::AwaitingReview,
            false,
            crate::plan::audit_sentinel::CompletionAuditFields {
                worker_branch: Some(worker_branch.to_string()),
                result_summary: Some(format!("approved dep {task_id}")),
                dispatched_base_oid: Some(base_oid.clone()),
                estimated_cost_micros: None,
                ..Default::default()
            },
        )
        .await
        .expect("completion audit");
        crate::plan::emit_approval_audit(
            Some(pm.as_ref()),
            &Some(issue_id.to_string()),
            feature_gate.as_ref(),
            plan_id,
            &delegation_id,
        )
        .await;
        pm.update_issue(
            issue_id,
            spur_pm::IssueUpdate {
                status: Some(pm.closed_status().to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("close dep task");
    }

    (pm, dep_issue_id, ready_issue_id)
}

#[tokio::test]
async fn tick_once_predicts_overlay_conflict_and_blocks_without_dispatch() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let plan_id = "PREDISPATCH-CONFLICT";
    let brain_session_id =
        spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-conflict".into()));
    let (pm, _dep_issue_id, ready_issue_id) =
        seed_ready_overlay_plan(dir.path(), plan_id, &brain_session_id).await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let config = ReconcilerConfig {
        repo_root: dir.path().to_path_buf(),
        predispatch_preview: super::PreviewStrategy::AlwaysConflict {
            dep_task_id: "X".into(),
            files: vec!["a.rs".into()],
        },
        ..Default::default()
    };
    let reconciler = Reconciler::new(
        config,
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id)),
        Some(plan_id.into()),
        pro_feature_gate(),
    );

    let did_work = reconciler.tick_once().await.expect("tick once");

    // `did_work` may be true because index-hygiene reconciliation can write.
    // The real no-dispatch invariant here is the channel check below.
    let _ = did_work;
    assert!(
        delegation_rx.try_recv().is_err(),
        "predicted conflict must not dispatch a worker"
    );
    let projected = reconciler
        .project_plan_from_beads(plan_id)
        .await
        .expect("project plan");
    let ready_task = projected
        .tasks
        .iter()
        .find(|entry| entry.spec.issue_id.as_deref() == Some(ready_issue_id.as_str()))
        .expect("ready task projection");
    assert!(
        matches!(
            &ready_task.status,
            crate::plan::PlanTaskStatus::BlockedOnSetupConflict { dep_task_id, files }
                if dep_task_id == "X" && files == &["a.rs".to_string()]
        ),
        "ready task should be blocked on predicted conflict, got {:?}",
        ready_task.status
    );
}

#[tokio::test]
async fn tick_once_with_clean_preview_dispatches_normally() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let plan_id = "PREDISPATCH-CLEAN";
    let brain_session_id = spur_acp::BrainSessionId::new(spur_acp::SessionId("brain-clean".into()));
    let (pm, _dep_issue_id, ready_issue_id) =
        seed_ready_overlay_plan(dir.path(), plan_id, &brain_session_id).await;
    let (delegation_tx, mut delegation_rx) =
        tokio::sync::mpsc::channel::<crate::DelegationRequest>(1);
    let config = ReconcilerConfig {
        repo_root: dir.path().to_path_buf(),
        predispatch_preview: super::PreviewStrategy::AlwaysClean,
        ..Default::default()
    };
    let reconciler = Reconciler::new(
        config,
        pm,
        Arc::new(Notify::new()),
        Some(test_dispatch_ctx(delegation_tx, brain_session_id)),
        Some(plan_id.into()),
        pro_feature_gate(),
    );

    let did_work = reconciler.tick_once().await.expect("tick once");
    let request = delegation_rx.recv().await.expect("dispatch request");

    assert!(did_work, "clean preview should allow normal dispatch");
    assert_eq!(request.issue_id.as_deref(), Some(ready_issue_id.as_str()));
    assert!(matches!(
        request.base,
        Some(crate::BaseSpec::WithOverlay { .. })
    ));
}

#[tokio::test]
async fn auto_merge_config_off_produces_zero_actions() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let repo = dir.path();
    let _beads = workspace_with_complete_epic(repo, "P1");
    let pm = pm_for_beads_repo(repo).await;

    let actions = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let automation = Arc::new(MockAutomation {
        actions: Arc::clone(&actions),
    });

    let mut reconciler = Reconciler::new(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        None,
        Some("P1".into()),
        pro_feature_gate(),
    );
    reconciler.set_auto_merge_approved_plans(false);
    reconciler.set_automation(automation);

    reconciler.tick_once().await.unwrap();

    let recorded = actions.lock().await;
    assert!(
        recorded.is_empty(),
        "config-off must produce zero automation actions, got: {:?}",
        *recorded
    );
}

/// Focused regression: when durable EpicCompletion audit emission fails
/// (e.g. disk-full / read-only database), the reconciler must suppress
/// merge_plan / create_pr even though the epic is closed and carries
/// integration-pending. Without this guard the old code would proceed
/// because it unconditionally appended a synthetic EpicCompletion to the
/// local audits vector.
#[tokio::test]
async fn failed_epic_completion_audit_suppresses_automation() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let repo = dir.path();
    let _beads = workspace_with_complete_epic(repo, "P1");
    let pm = pm_for_beads_repo(repo).await;

    // Make the beads database read-only so that add_comment (and therefore
    // emit_epic_completion_audit) fails. Some br versions refuse even read
    // commands against this fixture; that is still acceptable for this
    // regression because automation must not run when the durable audit
    // cannot be established.
    let db_path = repo.join(".beads").join("beads.db");
    let mut perms = std::fs::metadata(&db_path)
        .expect("db metadata")
        .permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&db_path, perms).expect("set readonly");

    let actions = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let automation = Arc::new(MockAutomation {
        actions: Arc::clone(&actions),
    });

    let mut reconciler = Reconciler::new(
        ReconcilerConfig::default(),
        pm,
        Arc::new(Notify::new()),
        None,
        Some("P1".into()),
        pro_feature_gate(),
    );
    reconciler.set_auto_merge_approved_plans(true);
    reconciler.set_automation(automation);

    match reconciler.tick_once().await {
        Ok(_) => {}
        Err(error)
            if error.to_string().contains("Permission denied")
                || error.to_string().contains("readonly")
                || error.to_string().contains("read-only") => {}
        Err(error) => panic!("unexpected tick_once error: {error:#}"),
    }

    let recorded = actions.lock().await;
    assert!(
        recorded.is_empty(),
        "failed epic-completion audit must suppress automation, got: {:?}",
        *recorded
    );
}

#[tokio::test]
async fn hybrid_journal_probe_disables_itself_when_missing() {
    let notify = Arc::new(Notify::new());
    let path = std::path::PathBuf::from("/nonexistent/path/.beads/journal");
    // The monitor must exit gracefully (not panic or hang) when the journal is absent.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        super::monitor_journal_appends(path, notify),
    )
    .await;
    assert!(
        result.is_ok(),
        "journal monitor must exit when path is missing, not hang"
    );
}

fn apply_label_delta(
    existing: &[String],
    add_labels: &[String],
    remove_labels: &[String],
) -> Vec<String> {
    let mut next = existing.to_vec();
    next.retain(|label| !remove_labels.contains(label));
    for label in add_labels {
        if !next.contains(label) {
            next.push(label.clone());
        }
    }
    next.sort();
    next.dedup();
    next
}

proptest! {
    #[test]
    fn plan_id_reconcile_converges_idempotently(
        expected in prop_oneof![
            Just(None),
            Just(Some(crate::plan::labels::plan_id("P1"))),
        ],
        has_expected in any::<bool>(),
        has_stale in any::<bool>(),
        junk in prop::collection::vec("[a-z0-9_-]{1,12}", 0..4),
    ) {
        let canonical = crate::plan::labels::plan_id("P1");
        let stale = crate::plan::labels::plan_id("P2");
        let mut existing = junk;
        if has_expected && expected.as_deref() == Some(canonical.as_str()) {
            existing.push(canonical.clone());
        }
        if has_stale {
            existing.push(stale.clone());
        }

        let mut add = Vec::new();
        let mut remove = Vec::new();
        let mut drift = Vec::new();
        super::reconcile_singleton_label(
            "plan_id",
            expected.clone(),
            existing.clone(),
            &mut add,
            &mut remove,
            &mut drift,
        );
        if expected.is_none() && has_stale {
            prop_assert!(remove.contains(&stale));
            prop_assert!(drift.iter().any(|event| event.direction == "stale"));
        }

        let converged = apply_label_delta(&existing, &add, &remove);
        let mut add2 = Vec::new();
        let mut remove2 = Vec::new();
        let mut drift2 = Vec::new();
        super::reconcile_singleton_label(
            "plan_id",
            expected,
            converged,
            &mut add2,
            &mut remove2,
            &mut drift2,
        );
        prop_assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
    }

    #[test]
    fn plan_task_id_reconcile_converges_idempotently(
        expected in prop_oneof![
            Just(None),
            Just(Some(crate::plan::labels::plan_task_id("T1"))),
        ],
        has_expected in any::<bool>(),
        has_stale in any::<bool>(),
        junk in prop::collection::vec("[a-z0-9_-]{1,12}", 0..4),
    ) {
        let canonical = crate::plan::labels::plan_task_id("T1");
        let stale = crate::plan::labels::plan_task_id("T2");
        let mut existing = junk;
        if has_expected && expected.as_deref() == Some(canonical.as_str()) {
            existing.push(canonical.clone());
        }
        if has_stale {
            existing.push(stale.clone());
        }

        let mut add = Vec::new();
        let mut remove = Vec::new();
        let mut drift = Vec::new();
        super::reconcile_singleton_label(
            "plan_task_id",
            expected.clone(),
            existing.clone(),
            &mut add,
            &mut remove,
            &mut drift,
        );
        if expected.is_none() && has_stale {
            prop_assert!(remove.contains(&stale));
            prop_assert!(drift.iter().any(|event| event.direction == "stale"));
        }

        let converged = apply_label_delta(&existing, &add, &remove);
        let mut add2 = Vec::new();
        let mut remove2 = Vec::new();
        let mut drift2 = Vec::new();
        super::reconcile_singleton_label(
            "plan_task_id",
            expected,
            converged,
            &mut add2,
            &mut remove2,
            &mut drift2,
        );
        prop_assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
    }

    #[test]
    fn agent_reconcile_converges_idempotently(
        expected in prop_oneof![
            Just(None),
            Just(Some(crate::plan::labels::agent("codex"))),
        ],
        has_expected in any::<bool>(),
        has_stale in any::<bool>(),
        junk in prop::collection::vec("[a-z0-9_-]{1,12}", 0..4),
    ) {
        let canonical = crate::plan::labels::agent("codex");
        let stale = crate::plan::labels::agent("gemini");
        let mut existing = junk;
        if has_expected && expected.as_deref() == Some(canonical.as_str()) {
            existing.push(canonical.clone());
        }
        if has_stale {
            existing.push(stale.clone());
        }

        let mut add = Vec::new();
        let mut remove = Vec::new();
        let mut drift = Vec::new();
        super::reconcile_singleton_label(
            "agent",
            expected.clone(),
            existing.clone(),
            &mut add,
            &mut remove,
            &mut drift,
        );
        if expected.is_none() && has_stale {
            prop_assert!(remove.contains(&stale));
            prop_assert!(drift.iter().any(|event| event.direction == "stale"));
        }

        let converged = apply_label_delta(&existing, &add, &remove);
        let mut add2 = Vec::new();
        let mut remove2 = Vec::new();
        let mut drift2 = Vec::new();
        super::reconcile_singleton_label(
            "agent",
            expected,
            converged,
            &mut add2,
            &mut remove2,
            &mut drift2,
        );
        prop_assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
    }

    #[test]
    fn delegation_id_reconcile_converges_idempotently(
        expected in prop_oneof![
            Just(None),
            Just(Some(crate::plan::labels::delegation_id("d-current"))),
        ],
        has_expected in any::<bool>(),
        has_stale in any::<bool>(),
        has_legacy_stale in any::<bool>(),
        junk in prop::collection::vec("[a-z0-9_-]{1,12}", 0..4),
    ) {
        let canonical = crate::plan::labels::delegation_id("d-current");
        let stale = crate::plan::labels::delegation_id("d-prev");
        // Legacy non-prefixed form (no `spur:` prefix) — the form
        // `parse_plan_id` / `delegation_label_value` historically accepts.
        let legacy_stale = "delegation-id:d-legacy".to_string();
        let mut existing = junk;
        if has_expected && expected.as_deref() == Some(canonical.as_str()) {
            existing.push(canonical.clone());
        }
        if has_stale {
            existing.push(stale.clone());
        }
        if has_legacy_stale {
            existing.push(legacy_stale.clone());
        }

        let mut add = Vec::new();
        let mut remove = Vec::new();
        let mut drift = Vec::new();
        super::reconcile_singleton_label(
            "delegation_id",
            expected.clone(),
            existing.clone(),
            &mut add,
            &mut remove,
            &mut drift,
        );
        if expected.is_none() && has_stale {
            prop_assert!(remove.contains(&stale));
            prop_assert!(drift.iter().any(|event| event.direction == "stale"));
        }
        if expected.is_none() && has_legacy_stale {
            // The raw legacy label string must appear in remove_labels so the
            // backend can actually match and strip it (no silent canonicalization).
            prop_assert!(remove.contains(&legacy_stale));
        }

        let converged = apply_label_delta(&existing, &add, &remove);
        let mut add2 = Vec::new();
        let mut remove2 = Vec::new();
        let mut drift2 = Vec::new();
        super::reconcile_singleton_label(
            "delegation_id",
            expected,
            converged,
            &mut add2,
            &mut remove2,
            &mut drift2,
        );
        prop_assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
    }
}

#[test]
fn plan_complete_reconcile_missing_emits_drift_then_noops() {
    let existing = vec!["x".to_string()];
    let mut add = Vec::new();
    let mut remove = Vec::new();
    let mut drift = Vec::new();
    let mut buffers = super::LabelReconcileBuffers {
        add_labels: &mut add,
        remove_labels: &mut remove,
        drift_events: &mut drift,
    };
    super::reconcile_presence_label(
        "plan_complete",
        crate::plan::labels::PLAN_COMPLETE,
        |label| label == crate::plan::labels::PLAN_COMPLETE,
        true,
        &existing,
        &mut buffers,
    );
    assert!(add.contains(&crate::plan::labels::PLAN_COMPLETE.to_string()));
    assert!(drift.iter().any(|event| event.direction == "missing"));

    let converged = apply_label_delta(&existing, &add, &remove);
    let mut add2 = Vec::new();
    let mut remove2 = Vec::new();
    let mut drift2 = Vec::new();
    let mut buffers2 = super::LabelReconcileBuffers {
        add_labels: &mut add2,
        remove_labels: &mut remove2,
        drift_events: &mut drift2,
    };
    super::reconcile_presence_label(
        "plan_complete",
        crate::plan::labels::PLAN_COMPLETE,
        |label| label == crate::plan::labels::PLAN_COMPLETE,
        true,
        &converged,
        &mut buffers2,
    );
    assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
}

#[test]
fn plan_pending_reconcile_stale_emits_drift_then_noops() {
    let existing = vec![
        crate::plan::labels::PLAN_PENDING.to_string(),
        "x".to_string(),
    ];
    let mut add = Vec::new();
    let mut remove = Vec::new();
    let mut drift = Vec::new();
    let mut buffers = super::LabelReconcileBuffers {
        add_labels: &mut add,
        remove_labels: &mut remove,
        drift_events: &mut drift,
    };
    super::reconcile_presence_label(
        "plan_pending",
        crate::plan::labels::PLAN_PENDING,
        |label| label == crate::plan::labels::PLAN_PENDING,
        false,
        &existing,
        &mut buffers,
    );
    assert!(remove.contains(&crate::plan::labels::PLAN_PENDING.to_string()));
    assert!(drift.iter().any(|event| event.direction == "stale"));

    let converged = apply_label_delta(&existing, &add, &remove);
    let mut add2 = Vec::new();
    let mut remove2 = Vec::new();
    let mut drift2 = Vec::new();
    let mut buffers2 = super::LabelReconcileBuffers {
        add_labels: &mut add2,
        remove_labels: &mut remove2,
        drift_events: &mut drift2,
    };
    super::reconcile_presence_label(
        "plan_pending",
        crate::plan::labels::PLAN_PENDING,
        |label| label == crate::plan::labels::PLAN_PENDING,
        false,
        &converged,
        &mut buffers2,
    );
    assert!(add2.is_empty() && remove2.is_empty() && drift2.is_empty());
}

#[derive(Clone)]
struct ParentFallbackAdvanced {
    comments_by_issue: std::collections::HashMap<String, Vec<spur_pm::Comment>>,
}

#[async_trait::async_trait]
impl spur_pm::BeadsAdvanced for ParentFallbackAdvanced {
    async fn list_ready(
        &self,
        _filter: spur_pm::ReadyFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        Ok(Vec::new())
    }

    async fn list_comments(&self, issue_id: &str) -> anyhow::Result<Vec<spur_pm::Comment>> {
        Ok(self
            .comments_by_issue
            .get(issue_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn add_comment(&self, _issue_id: &str, _body: &str) -> anyhow::Result<String> {
        Ok("c1".to_string())
    }

    async fn remove_dependency(&self, _issue_id: &str, _depends_on_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn dep_cycles(&self) -> anyhow::Result<Vec<spur_pm::DependencyCycle>> {
        Ok(Vec::new())
    }
}

struct ParentFallbackPm {
    issue: spur_pm::Issue,
    graph: spur_pm::graph::DependencyGraph,
    advanced: ParentFallbackAdvanced,
}

#[async_trait::async_trait]
impl crate::plan::PmLike for ParentFallbackPm {
    async fn get_issue(&self, id: &str) -> anyhow::Result<spur_pm::Issue> {
        if id == self.issue.id {
            return Ok(self.issue.clone());
        }
        anyhow::bail!("unknown issue id: {id}");
    }

    async fn update_issue(&self, _id: &str, _update: spur_pm::IssueUpdate) -> anyhow::Result<()> {
        Ok(())
    }

    async fn issue_subgraph_json(
        &self,
        _id: &str,
    ) -> anyhow::Result<spur_pm::graph::DependencyGraph> {
        Ok(self.graph.clone())
    }

    fn closed_status(&self) -> &str {
        "closed"
    }

    fn advanced(&self) -> Option<&dyn spur_pm::BeadsAdvanced> {
        Some(&self.advanced)
    }
}

fn parent_fallback_graph(parents: &[&str]) -> spur_pm::graph::DependencyGraph {
    spur_pm::graph::DependencyGraph {
        format: Some("json".to_string()),
        nodes: parents.len() + 1,
        edges: parents.len(),
        adjacency: Some(spur_pm::graph::AdjacencyData {
            nodes: Vec::new(),
            edges: Some(
                parents
                    .iter()
                    .map(|parent| spur_pm::graph::GraphEdge {
                        from: (*parent).to_string(),
                        to: "bd-child".to_string(),
                        edge_type: Some("parent-child".to_string()),
                    })
                    .collect(),
            ),
        }),
        ..Default::default()
    }
}

fn parent_fallback_issue(blocked_by: Vec<String>) -> spur_pm::Issue {
    spur_pm::Issue {
        id: "bd-child".to_string(),
        source: spur_pm::PmSource::Beads,
        title: "Child".to_string(),
        body: String::new(),
        status: "open".to_string(),
        labels: vec![],
        assignee: None,
        url: "https://example.invalid/bd-child".to_string(),
        priority: None,
        issue_type: Some("task".to_string()),
        blocked_by,
        due_at: None,
        source_system: None,
        source_repo: None,
        external_ref: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn plan_submit_comment(plan_id: &str, epic_id: &str) -> spur_pm::Comment {
    spur_pm::Comment {
        id: format!("c-{epic_id}"),
        body: crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::PlanSubmit {
                plan_id: plan_id.to_string(),
                epic_issue_id: epic_id.to_string(),
                task_ids: Vec::new(),
                base_snapshot_branch: None,
                base_snapshot_oid: None,
                execution_mode: None,
                brain_session_id: None,
                explicit_base: None,
            },
        ),
        actor: "tester".to_string(),
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn expected_plan_id_from_parent_epic_uses_structural_parent_not_effective_blocker() {
    let issue_summary = spur_pm::IssueSummary {
        id: "bd-child".to_string(),
        source: spur_pm::PmSource::Beads,
        title: "Child".to_string(),
        status: "open".to_string(),
        labels: vec![],
        url: "https://example.invalid/bd-child".to_string(),
        priority: None,
        issue_type: Some("task".to_string()),
        assignee: None,
        description: None,
    };
    let mut comments_by_issue = std::collections::HashMap::new();
    comments_by_issue.insert(
        "bd-parent-a".to_string(),
        vec![plan_submit_comment("PLAN-A", "bd-parent-a")],
    );
    comments_by_issue.insert(
        "bd-parent-b".to_string(),
        vec![plan_submit_comment("PLAN-B", "bd-parent-b")],
    );
    let feature_gate = pro_feature_gate();

    let pm = Arc::new(ParentFallbackPm {
        issue: parent_fallback_issue(vec!["bd-parent-b".to_string()]),
        graph: parent_fallback_graph(&["bd-parent-a"]),
        advanced: ParentFallbackAdvanced { comments_by_issue },
    });
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone() as Arc<dyn crate::plan::PmLike>,
        Arc::new(Notify::new()),
        None,
        None,
        feature_gate,
    );
    let resolved = reconciler
        .expected_plan_id_from_parent_epic(
            crate::plan::PmLike::advanced(pm.as_ref()).expect("advanced"),
            &issue_summary,
        )
        .await
        .expect("structural parent fallback");

    assert_eq!(resolved.as_deref(), Some("PLAN-A"));
}

#[tokio::test]
async fn expected_plan_id_from_parent_epic_rejects_ambiguous_structural_parents() {
    let issue_summary = spur_pm::IssueSummary {
        id: "bd-child".to_string(),
        source: spur_pm::PmSource::Beads,
        title: "Child".to_string(),
        status: "open".to_string(),
        labels: vec![],
        url: "https://example.invalid/bd-child".to_string(),
        priority: None,
        issue_type: Some("task".to_string()),
        assignee: None,
        description: None,
    };
    let pm = Arc::new(ParentFallbackPm {
        issue: parent_fallback_issue(Vec::new()),
        graph: parent_fallback_graph(&["bd-parent-a", "bd-parent-b"]),
        advanced: ParentFallbackAdvanced {
            comments_by_issue: std::collections::HashMap::new(),
        },
    });
    let reconciler = Reconciler::new_with_pm_like(
        ReconcilerConfig::default(),
        pm.clone() as Arc<dyn crate::plan::PmLike>,
        Arc::new(Notify::new()),
        None,
        None,
        pro_feature_gate(),
    );
    let error = reconciler
        .expected_plan_id_from_parent_epic(
            crate::plan::PmLike::advanced(pm.as_ref()).expect("advanced"),
            &issue_summary,
        )
        .await
        .expect_err("multiple structural parents must fail safely");

    assert!(error.to_string().contains("multiple structural parents"));
}

#[tokio::test]
async fn tick_once_retains_agent_and_plan_task_id_for_empty_context_files_task() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path();
    let empty = spur_pm::test_workspace::TestBeadsWorkspace::init();
    let beads_dir = repo.join(".beads");
    std::fs::create_dir(&beads_dir).expect("create test .beads directory");
    empty.copy_db_to(&beads_dir);
    let pm = pm_for_beads_repo(repo).await;
    let feature_gate = pro_feature_gate();

    let plan_id = "P-EMPTY-CONTEXT";
    let sg = crate::build_epic_subgraph(
        pm.as_ref(),
        feature_gate.as_ref(),
        plan_id,
        "Empty Context Plan",
        Some("empty-context regression"),
        &[crate::plan::PlanTask {
            task_id: "T1".to_string(),
            agent: "codex".to_string(),
            profile: None,
            skills: None,
            model: None,
            effort: None,
            config_overrides: None,
            task: "do work".to_string(),
            depends_on: vec![],
            issue_id: None,
            issue_title: None,
            context_files: Vec::new(),
            planned_write_files: None,
        }],
    )
    .await
    .expect("persist plan");
    crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        feature_gate.as_ref(),
    )
    .expect("test feature gate should allow beads advanced");
    let adv = pm.advanced().expect("advanced");
    crate::emit_plan_submit_audit(adv, plan_id, &sg, crate::PlanSubmitAuditContext::default())
        .await;
    let child_id = sg.task_map.get("T1").cloned().expect("task map for T1");
    adv.add_comment(
        &child_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::Dispatch {
                delegation_id: "del-T1".to_string(),
                worker: "codex".to_string(),
                attempt: 1,
            },
        ),
    )
    .await
    .expect("dispatch audit");

    let reconciler = Reconciler::new(
        ReconcilerConfig::default(),
        Arc::clone(&pm),
        Arc::new(Notify::new()),
        None,
        Some(plan_id.to_string()),
        feature_gate,
    );
    let _ = reconciler.tick_once().await.expect("tick once");

    let child = pm.get_issue(&child_id).await.expect("child issue");
    assert!(
        child
            .labels
            .iter()
            .any(|label| label == &crate::plan::labels::agent("codex")),
        "child must retain spur:agent:* after tick"
    );
    assert!(
        child
            .labels
            .iter()
            .any(|label| label == &crate::plan::labels::plan_task_id("T1")),
        "child must retain spur:plan-task-id:* after tick"
    );
}

#[tokio::test]
async fn tick_once_strips_plan_complete_when_plan_submit_audit_is_absent() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path();
    let empty = spur_pm::test_workspace::TestBeadsWorkspace::init();
    let beads_dir = repo.join(".beads");
    std::fs::create_dir(&beads_dir).expect("create test .beads directory");
    empty.copy_db_to(&beads_dir);
    let pm = pm_for_beads_repo(repo).await;
    let feature_gate = pro_feature_gate();

    let issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "orphan plan-complete".to_string(),
            issue_type: Some("epic".to_string()),
            labels: vec![crate::plan::labels::PLAN_COMPLETE.to_string()],
            ..Default::default()
        })
        .await
        .expect("create issue");
    crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        feature_gate.as_ref(),
    )
    .expect("test feature gate should allow beads advanced");
    let adv = pm.advanced().expect("advanced");
    adv.add_comment(
        &issue_id,
        &crate::plan::audit_sentinel::encode_comment(
            &crate::plan::audit_sentinel::AuditSentinelKind::Dispatch {
                delegation_id: "del-orphan".to_string(),
                worker: "codex".to_string(),
                attempt: 1,
            },
        ),
    )
    .await
    .expect("dispatch audit");

    let reconciler = Reconciler::new(
        ReconcilerConfig::default(),
        Arc::clone(&pm),
        Arc::new(Notify::new()),
        None,
        None,
        feature_gate,
    );
    let _ = reconciler.tick_once().await.expect("tick once");

    let issue = pm.get_issue(&issue_id).await.expect("issue");
    assert!(
        !issue
            .labels
            .iter()
            .any(|label| label == crate::plan::labels::PLAN_COMPLETE),
        "spur:plan-complete must be stripped without a PlanSubmit audit"
    );
}

#[path = "hygiene_tests.rs"]
mod hygiene_tests;
