//! Behavioral tests exercise the real adapter and count comment reads at the boundary.
use super::*;
use spur_pm::BeadsAdvanced;
use std::sync::Mutex;

struct CountingPm {
    inner: Arc<spur_pm::PmService>,
    reads: Mutex<Vec<String>>,
    graph_reads: AtomicUsize,
    fail: Mutex<Option<String>>,
    comment_on_write: Mutex<Option<(String, String)>>,
    unsupported: std::sync::atomic::AtomicBool,
    reseed: std::sync::atomic::AtomicBool,
    feed_error: std::sync::atomic::AtomicBool,
    pause: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
}

#[async_trait::async_trait]
impl spur_pm::BeadsAdvanced for CountingPm {
    async fn hygiene_changes(
        &self,
        since: Option<&spur_pm::advanced::HygieneCursor>,
    ) -> anyhow::Result<Option<spur_pm::advanced::HygieneBatch>> {
        anyhow::ensure!(
            !self.feed_error.load(Ordering::SeqCst),
            "injected feed error"
        );
        if self.unsupported.load(Ordering::SeqCst) {
            return Ok(None);
        }
        self.inner
            .advanced()
            .unwrap()
            .hygiene_changes(if self.reseed.swap(false, Ordering::SeqCst) {
                None
            } else {
                since
            })
            .await
    }

    async fn list_ready(
        &self,
        f: spur_pm::ReadyFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        self.inner.advanced().unwrap().list_ready(f).await
    }
    async fn list_comments(&self, id: &str) -> anyhow::Result<Vec<spur_pm::Comment>> {
        self.reads.lock().unwrap().push(id.into());
        let pause = self.pause.lock().unwrap().take();
        if let Some((entered, release)) = pause {
            entered.notify_one();
            release.notified().await;
        }
        anyhow::ensure!(
            self.fail.lock().unwrap().as_deref() != Some(id),
            "injected comment failure"
        );
        self.inner.advanced().unwrap().list_comments(id).await
    }
    async fn add_comment(&self, id: &str, body: &str) -> anyhow::Result<String> {
        self.inner.advanced().unwrap().add_comment(id, body).await
    }
    async fn remove_dependency(&self, id: &str, parent: &str) -> anyhow::Result<()> {
        self.inner
            .advanced()
            .unwrap()
            .remove_dependency(id, parent)
            .await
    }
    async fn dep_cycles(&self) -> anyhow::Result<Vec<spur_pm::DependencyCycle>> {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl PmLike for CountingPm {
    async fn get_issue(&self, id: &str) -> anyhow::Result<spur_pm::Issue> {
        self.inner.get_issue(id).await
    }
    async fn list_issues(
        &self,
        f: spur_pm::IssueFilter,
    ) -> anyhow::Result<Vec<spur_pm::IssueSummary>> {
        self.inner.list_issues(f).await
    }
    async fn update_issue(&self, id: &str, u: spur_pm::IssueUpdate) -> anyhow::Result<()> {
        let raced = self.comment_on_write.lock().unwrap().take();
        if let Some((id, body)) = raced {
            self.inner
                .advanced()
                .unwrap()
                .add_comment(&id, &body)
                .await?;
        }
        self.inner.update_issue(id, u).await
    }
    async fn issue_subgraph_json(
        &self,
        id: &str,
    ) -> anyhow::Result<spur_pm::graph::DependencyGraph> {
        self.graph_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.issue_subgraph_json(id).await
    }
    fn advanced(&self) -> Option<&dyn spur_pm::BeadsAdvanced> {
        Some(self)
    }
    fn closed_status(&self) -> &str {
        "closed"
    }
}

async fn fixture() -> (tempfile::TempDir, Arc<CountingPm>, Reconciler) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".beads")).unwrap();
    let adapter = spur_pm::beads_crate::BeadsCrateAdapter::open(
        &dir.path().join(".beads"),
        Default::default(),
    )
    .await
    .unwrap();
    drop(adapter);
    let pm = Arc::new(CountingPm {
        inner: pm_for_beads_repo(dir.path()).await,
        reads: Mutex::new(vec![]),
        graph_reads: AtomicUsize::new(0),
        fail: Mutex::new(None),
        comment_on_write: Mutex::new(None),
        unsupported: Default::default(),
        reseed: Default::default(),
        feed_error: Default::default(),
        pause: Mutex::new(None),
    });
    let r = Reconciler::new_with_pm_like(
        Default::default(),
        pm.clone(),
        Arc::new(Notify::new()),
        None,
        None,
        pro_feature_gate(),
    );
    (dir, pm, r)
}
async fn issue(pm: &CountingPm, title: &str) -> String {
    pm.inner
        .create_issue(spur_pm::IssueCreate {
            title: title.into(),
            ..Default::default()
        })
        .await
        .unwrap()
}
fn take_reads(pm: &CountingPm) -> Vec<String> {
    std::mem::take(&mut *pm.reads.lock().unwrap())
}

#[tokio::test]
async fn hygiene_unchanged_skips_comments_and_leaf_edit_stays_local() {
    let (_dir, pm, r) = fixture().await;
    let a = issue(&pm, "A").await;
    let b = issue(&pm, "B").await;
    assert!(!r.run_index_hygiene_sweep().await.unwrap());
    assert_eq!(take_reads(&pm).len(), 2);
    assert!(!r.run_index_hygiene_sweep().await.unwrap());
    assert_eq!(
        take_reads(&pm),
        Vec::<String>::new(),
        "unchanged hygiene must not reread comments"
    );
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&a, "ordinary comment")
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(take_reads(&pm), vec![a.clone()]);
    pm.inner
        .update_issue(
            &b,
            spur_pm::IssueUpdate {
                priority: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(take_reads(&pm), vec![b]);
}

#[tokio::test]
async fn hygiene_pending_grace_expires_without_db_change_or_comment_reread() {
    let (_dir, pm, mut r) = fixture().await;
    let a = issue(&pm, "pending dispatch").await;
    pm.inner
        .update_issue(
            &a,
            spur_pm::IssueUpdate {
                add_labels: vec![crate::plan::labels::delegation_id("pending")],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let updated = pm.inner.get_issue(&a).await.unwrap().updated_at;
    r.set_clock(Arc::new(FixedClock {
        now: updated.into(),
    }));
    assert!(!r.run_index_hygiene_sweep().await.unwrap());
    assert_eq!(take_reads(&pm), vec![a.clone()]);
    r.set_clock(Arc::new(FixedClock {
        now: SystemTime::from(updated) + r.config.label_only_dispatch_grace,
    }));
    assert!(r.run_index_hygiene_sweep().await.unwrap());
    assert!(
        take_reads(&pm).is_empty(),
        "pending empty audits must survive unchanged timer passes"
    );
    assert!(!pm
        .inner
        .get_issue(&a)
        .await
        .unwrap()
        .labels
        .iter()
        .any(|l| delegation_label_value(l).is_some()));
}

#[tokio::test]
async fn hygiene_partial_failure_and_comment_during_label_write_are_retried() {
    let (_dir, pm, r) = fixture().await;
    let a = issue(&pm, "A").await;
    let b = issue(&pm, "B").await;
    pm.inner
        .update_issue(
            &b,
            spur_pm::IssueUpdate {
                priority: Some(4),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Priority gives the successful repair a deterministic position before failure.
    pm.inner
        .update_issue(
            &a,
            spur_pm::IssueUpdate {
                priority: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&a, &plan_submit_comment("P1", &a).body)
        .await
        .unwrap();
    *pm.fail.lock().unwrap() = Some(b.clone());
    *pm.comment_on_write.lock().unwrap() = Some((a.clone(), plan_submit_comment("P2", &a).body));
    let error = r.run_index_hygiene_sweep().await.unwrap_err();
    assert!(
        error.to_string().contains("injected comment failure"),
        "{error:#}"
    );
    assert!(pm
        .inner
        .get_issue(&a)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("P1")));
    *pm.fail.lock().unwrap() = None;
    take_reads(&pm);
    assert!(r.run_index_hygiene_sweep().await.unwrap());
    assert!(take_reads(&pm).contains(&b));
    assert!(pm
        .inner
        .get_issue(&a)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("P2")));
    // Drain self writes, then require an unchanged pass to be read-free.
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    r.run_index_hygiene_sweep().await.unwrap();
    assert!(take_reads(&pm).is_empty());
}

#[tokio::test]
async fn hygiene_membership_reseed_unsupported_and_feed_errors() {
    let (_dir, pm, r) = fixture().await;
    let a = issue(&pm, "A").await;
    let b = issue(&pm, "B").await;
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    pm.inner
        .update_issue(
            &a,
            spur_pm::IssueUpdate {
                status: Some("closed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert!(
        take_reads(&pm).is_empty(),
        "exit must evict without reading comments"
    );
    pm.inner
        .update_issue(
            &a,
            spur_pm::IssueUpdate {
                status: Some("open".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    pm.feed_error.store(true, Ordering::SeqCst);
    assert!(r
        .run_index_hygiene_sweep()
        .await
        .unwrap_err()
        .to_string()
        .contains("injected feed error"));
    pm.feed_error.store(false, Ordering::SeqCst);
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(take_reads(&pm), vec![a.clone()]);
    pm.reseed.store(true, Ordering::SeqCst);
    r.run_index_hygiene_sweep().await.unwrap();
    let reads = take_reads(&pm);
    assert!(reads.contains(&a) && reads.contains(&b));
    pm.unsupported.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        r.run_index_hygiene_sweep().await.unwrap();
        assert_eq!(
            take_reads(&pm).len(),
            2,
            "unsupported always uses the full path"
        );
    }
}

#[tokio::test]
async fn hygiene_successful_write_does_not_ack_a_concurrent_comment() {
    let (_dir, pm, r) = fixture().await;
    let a = issue(&pm, "A").await;
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&a, &plan_submit_comment("P1", &a).body)
        .await
        .unwrap();
    *pm.comment_on_write.lock().unwrap() = Some((a.clone(), plan_submit_comment("P2", &a).body));
    assert!(r.run_index_hygiene_sweep().await.unwrap());
    assert!(pm
        .inner
        .get_issue(&a)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("P1")));
    assert!(r.run_index_hygiene_sweep().await.unwrap());
    assert!(pm
        .inner
        .get_issue(&a)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("P2")));
}

#[tokio::test]
async fn hygiene_overlapping_sweeps_serialize_and_cancelled_sweeps_retry() {
    for cancel in [false, true] {
        let (_dir, pm, r) = fixture().await;
        let a = issue(&pm, "A").await;
        let r = Arc::new(r);
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        *pm.pause.lock().unwrap() = Some((entered.clone(), release.clone()));
        let first = tokio::spawn({
            let r = r.clone();
            async move { r.run_index_hygiene_sweep().await }
        });
        entered.notified().await;
        let second = tokio::spawn({
            let r = r.clone();
            async move { r.run_index_hygiene_sweep().await }
        });
        if cancel {
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
        } else {
            release.notify_one();
            first.await.unwrap().unwrap();
        }
        second.await.unwrap().unwrap();
        assert_eq!(take_reads(&pm), vec![a; if cancel { 2 } else { 1 }]);
    }
}

#[tokio::test]
async fn hygiene_closed_parent_changes_expand_to_child_and_direct_audit_wins() {
    let (_dir, pm, r) = fixture().await;
    let parent = issue(&pm, "parent").await;
    let child = pm
        .inner
        .create_issue(spur_pm::IssueCreate {
            title: "child".into(),
            parent: Some(parent.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let leaf = issue(&pm, "unrelated").await;
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&parent, &plan_submit_comment("P1", &parent).body)
        .await
        .unwrap();
    let dispatch = crate::plan::audit_sentinel::encode_comment(
        &crate::plan::audit_sentinel::AuditSentinelKind::Dispatch {
            delegation_id: "d1".into(),
            worker: "codex".into(),
            attempt: 1,
        },
    );
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&child, &dispatch)
        .await
        .unwrap();
    pm.inner
        .update_issue(
            &parent,
            spur_pm::IssueUpdate {
                status: Some("closed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&parent, &plan_submit_comment("P2", &parent).body)
        .await
        .unwrap();
    assert!(r.run_index_hygiene_sweep().await.unwrap());
    let reads = take_reads(&pm);
    assert!(reads.contains(&child) && reads.contains(&parent));
    assert!(!reads.contains(&leaf));
    assert!(pm
        .inner
        .get_issue(&child)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("P2")));
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&child, &plan_submit_comment("DIRECT", &child).body)
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&parent, &plan_submit_comment("P3", &parent).body)
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(
        take_reads(&pm),
        vec![child.clone()],
        "direct audit avoids parent comment reads"
    );
    assert_eq!(
        pm.graph_reads.load(Ordering::SeqCst),
        0,
        "supported deltas use indexed parent IDs, not graph snapshots"
    );
    assert!(pm
        .inner
        .get_issue(&child)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_id("DIRECT")));
}

#[tokio::test]
async fn hygiene_unchanged_database_still_runs_due_loop_on_tick() {
    let (_dir, pm, mut r) = fixture().await;
    let spec = crate::plan::loops::spec::LoopSpec {
        loop_id: "timed".into(),
        goal: "time driven".into(),
        pattern: None,
        cadence_secs: 60,
        autonomy: crate::plan::loops::spec::AutonomyLevel::L1,
        template: serde_json::json!({"tasks": []}),
        governors: Default::default(),
        escalation: None,
    };
    pm.inner
        .create_issue(spur_pm::IssueCreate {
            title: "Timed loop".into(),
            description: Some(spec.to_sentinel_body()),
            issue_type: Some(crate::plan::loops::LOOP_ISSUE_TYPE.into()),
            labels: vec![
                crate::plan::labels::loop_id_label("timed"),
                crate::plan::labels::loop_next_run_label(200),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let brain = spur_acp::BrainSessionId::new(spur_acp::SessionId("timer-brain".into()));
    let (dispatch, continuations) = test_dispatch_ctx_with_recording(tx, brain, None);
    r.dispatch = Some(dispatch.into_dispatch());
    r.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
    }));
    r.tick_once().await.unwrap();
    assert!(continuations.lock().unwrap().is_empty());
    let cursor = pm
        .inner
        .advanced()
        .unwrap()
        .hygiene_changes(None)
        .await
        .unwrap()
        .unwrap()
        .cursor;
    r.tick_once().await.unwrap();
    let unchanged = pm
        .inner
        .advanced()
        .unwrap()
        .hygiene_changes(Some(&cursor))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.cursor, cursor,
        "pre-due tick leaves database unchanged"
    );
    r.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(200),
    }));
    r.tick_once().await.unwrap();
    assert_eq!(
        continuations.lock().unwrap().len(),
        1,
        "due loop must run despite unchanged hygiene revision"
    );
}

#[tokio::test]
async fn hygiene_unchanged_database_still_renews_expired_owner_lease() {
    let (_dir, pm, mut r) = fixture().await;
    let owner = crate::plan::loops::LOOP_RUNTIME_OWNER_ID;
    let epic = pm
        .inner
        .create_issue(spur_pm::IssueCreate {
            title: "owned epic".into(),
            issue_type: Some("epic".into()),
            labels: vec![
                crate::plan::labels::PLAN_COMPLETE.into(),
                crate::plan::labels::plan_owner(owner),
                format!("{}l3", crate::plan::labels::AUTONOMY_PREFIX),
                crate::plan::labels::plan_owner_lease_expires_at(200),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    r.config.plan_scope = PlanScope::SystemL3Only;
    r.config.dispatch_lease_duration = Duration::from_secs(60);
    r.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
    }));
    r.run_index_hygiene_sweep().await.unwrap();
    assert!(!r.reconcile_plan_owner_leases().await.unwrap());
    let cursor = pm
        .inner
        .advanced()
        .unwrap()
        .hygiene_changes(None)
        .await
        .unwrap()
        .unwrap()
        .cursor;
    r.run_index_hygiene_sweep().await.unwrap();
    let unchanged = pm
        .inner
        .advanced()
        .unwrap()
        .hygiene_changes(Some(&cursor))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.cursor, cursor);
    r.set_clock(Arc::new(FixedClock {
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(201),
    }));
    assert!(r.reconcile_plan_owner_leases().await.unwrap());
    assert!(pm
        .inner
        .get_issue(&epic)
        .await
        .unwrap()
        .labels
        .contains(&crate::plan::labels::plan_owner_lease_expires_at(261)));
}

#[tokio::test]
async fn hygiene_pending_empty_audits_invalidate_on_comments_and_eligibility_exit() {
    let (_dir, pm, mut r) = fixture().await;
    let a = issue(&pm, "audit arrives during grace").await;
    let b = issue(&pm, "closes during grace").await;
    for id in [&a, &b] {
        pm.inner
            .update_issue(
                id,
                spur_pm::IssueUpdate {
                    add_labels: vec![crate::plan::labels::delegation_id("d1")],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let now = SystemTime::now();
    r.set_clock(Arc::new(FixedClock { now }));
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(take_reads(&pm).len(), 2);
    let dispatch = crate::plan::audit_sentinel::encode_comment(
        &crate::plan::audit_sentinel::AuditSentinelKind::Dispatch {
            delegation_id: "d1".into(),
            worker: "codex".into(),
            attempt: 1,
        },
    );
    pm.inner
        .advanced()
        .unwrap()
        .add_comment(&a, &dispatch)
        .await
        .unwrap();
    pm.inner
        .update_issue(
            &b,
            spur_pm::IssueUpdate {
                status: Some("closed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_eq!(take_reads(&pm), vec![a.clone()]);
    r.set_clock(Arc::new(FixedClock {
        now: now + r.config.label_only_dispatch_grace + Duration::from_secs(1),
    }));
    assert!(!r.run_index_hygiene_sweep().await.unwrap());
    assert!(take_reads(&pm).is_empty());
    for id in [&a, &b] {
        assert!(pm
            .inner
            .get_issue(id)
            .await
            .unwrap()
            .labels
            .contains(&crate::plan::labels::delegation_id("d1")));
    }
}

async fn parent_donation_fixture() -> (
    tempfile::TempDir,
    Arc<CountingPm>,
    Reconciler,
    String,
    String,
) {
    let (dir, pm, r) = fixture().await;
    let parent = issue(&pm, "parent").await;
    let child = pm
        .inner
        .create_issue(spur_pm::IssueCreate {
            title: "child".into(),
            parent: Some(parent.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    pm.add_comment(&parent, &plan_submit_comment("P1", &parent).body)
        .await
        .unwrap();
    let dispatch = crate::plan::audit_sentinel::encode_comment(
        &crate::plan::audit_sentinel::AuditSentinelKind::Dispatch {
            delegation_id: "d1".into(),
            worker: "codex".into(),
            attempt: 1,
        },
    );
    pm.add_comment(&child, &dispatch).await.unwrap();
    pm.inner
        .update_issue(
            &parent,
            spur_pm::IssueUpdate {
                status: Some("closed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_child_plan(&pm, &child, Some("P1")).await;
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    (dir, pm, r, parent, child)
}

async fn assert_child_plan(pm: &CountingPm, child: &str, expected: Option<&str>) {
    let issue = pm.inner.get_issue(child).await.unwrap();
    let plans: Vec<_> = issue
        .labels
        .iter()
        .filter(|label| crate::plan::labels::parse_plan_id(label).is_some())
        .cloned()
        .collect();
    let expected: Vec<_> = expected
        .map(crate::plan::labels::plan_id)
        .into_iter()
        .collect();
    assert_eq!(
        plans, expected,
        "child must reflect eligible structural parent's audit"
    );
}

#[tokio::test]
async fn hygiene_review_excluded_parent_removes_inherited_plan() {
    for sql in [
        "UPDATE issues SET status='tombstone' WHERE id=?1",
        "UPDATE issues SET status='ToMbStOnE',closed_at=NULL WHERE id=?1",
        "UPDATE issues SET is_template=1 WHERE id=?1",
        "DELETE FROM issues WHERE id=?1",
    ] {
        let (dir, pm, r, parent, child) = parent_donation_fixture().await;
        let db = rusqlite::Connection::open(dir.path().join(".beads/beads.db")).unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        db.execute(sql, [&parent]).unwrap();
        r.run_index_hygiene_sweep().await.unwrap();
        assert_child_plan(&pm, &child, None).await;
        assert_eq!(
            take_reads(&pm),
            vec![child.clone()],
            "excluded parent must not donate: {sql}"
        );
        assert_eq!(pm.graph_reads.load(Ordering::SeqCst), 0);
        r.run_index_hygiene_sweep().await.unwrap();
        take_reads(&pm);
        r.run_index_hygiene_sweep().await.unwrap();
        assert!(take_reads(&pm).is_empty());
    }
}

#[tokio::test]
async fn hygiene_review_mixed_case_parent_comment_repairs_child() {
    let (dir, pm, r, parent, child) = parent_donation_fixture().await;
    let db = rusqlite::Connection::open(dir.path().join(".beads/beads.db")).unwrap();
    db.execute(
        "UPDATE dependencies SET type='PARENT-CHILD' WHERE issue_id=?1",
        [&child],
    )
    .unwrap();
    // Drain the structural mutation and its repairs so only the parent comment
    // can select the child on the measured pass below.
    r.run_index_hygiene_sweep().await.unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    take_reads(&pm);
    pm.add_comment(&parent, &plan_submit_comment("P2", &parent).body)
        .await
        .unwrap();
    r.run_index_hygiene_sweep().await.unwrap();
    assert_child_plan(&pm, &child, Some("P2")).await;
    let reads = take_reads(&pm);
    assert!(reads.contains(&child) && reads.contains(&parent));
    assert_eq!(pm.graph_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn hygiene_review_mixed_case_structural_changes_repair_child() {
    let (dir, pm, r, parent, child) = parent_donation_fixture().await;
    let db = rusqlite::Connection::open(dir.path().join(".beads/beads.db")).unwrap();
    for (sql, expected) in [
        ("DELETE FROM dependencies WHERE issue_id=?1", None),
        (
            "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES(?1,?2,'PaReNt-ChIlD')",
            Some("P1"),
        ),
        (
            "UPDATE dependencies SET type='blocks' WHERE issue_id=?1",
            None,
        ),
        (
            "UPDATE dependencies SET type='PARENT-CHILD' WHERE issue_id=?1",
            Some("P1"),
        ),
        ("DELETE FROM dependencies WHERE issue_id=?1", None),
    ] {
        let mut statement = db.prepare(sql).unwrap();
        if statement.parameter_count() == 2 {
            statement.execute([&child, &parent]).unwrap();
        } else {
            statement.execute([&child]).unwrap();
        }
        r.run_index_hygiene_sweep().await.unwrap();
        assert_child_plan(&pm, &child, expected).await;
        // Drain label writes before the next raw structural mutation.
        r.run_index_hygiene_sweep().await.unwrap();
        take_reads(&pm);
        r.run_index_hygiene_sweep().await.unwrap();
        assert!(take_reads(&pm).is_empty(), "unchanged after {sql}");
    }
    assert_eq!(pm.graph_reads.load(Ordering::SeqCst), 0);
}
