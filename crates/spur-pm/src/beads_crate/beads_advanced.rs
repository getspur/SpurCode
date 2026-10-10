use std::str::FromStr;

use async_trait::async_trait;

use crate::advanced::{
    BeadsAdvanced, Comment, CommentId, DependencyCycle, ReadyFilter, ResolvedDepHint,
};
use crate::beads_crate::adapter::BeadsCrateAdapter;
use crate::beads_crate::issue_tracker::br_to_pm_summary;
use crate::ingest::{dep_hints, watermark};
use crate::types::IssueSummary;

impl BeadsCrateAdapter {
    /// Look up the `spur:plan-id:<plan-id>` label on an epic, if any.
    ///
    /// Returns `Ok(None)` for non-epic issues (the label is only meaningful
    /// on epics) and for epics that carry no `spur:plan-id:` label.
    /// Returns `Err` if the issue does not exist.
    ///
    /// Mirrors `BeadsAdapter::plan_id_label_for_epic` in `crates/spur-pm/
    /// src/beads.rs`; this is the inherent-method counterpart used by
    /// `PmService` (see `crates/spur-pm/src/service.rs:209`).
    pub async fn plan_id_label_for_epic(&self, id: &str) -> anyhow::Result<Option<String>> {
        let id = id.to_string();
        self.read(move |s| {
            let issue = s
                .get_issue(&id)?
                .ok_or_else(|| anyhow::anyhow!("issue {id} not found"))?;
            if !issue.issue_type.to_string().eq_ignore_ascii_case("epic") {
                return Ok(None);
            }

            Ok(s.get_labels(&id)?
                .into_iter()
                .find(|label| label.starts_with("spur:plan-id:")))
        })
        .await
    }
}

#[async_trait]
impl BeadsAdvanced for BeadsCrateAdapter {
    async fn hygiene_changes(
        &self,
        since: Option<&crate::advanced::HygieneCursor>,
    ) -> anyhow::Result<Option<crate::advanced::HygieneBatch>> {
        if !cfg!(unix) {
            return Ok(None);
        }
        let since = since.cloned();
        let reads = std::sync::Arc::clone(&self.issue_reads);
        let metrics = std::sync::Arc::clone(&self.metrics);
        self.db
            .submit_summary_read(move |_| {
                let mut reads = reads
                    .lock()
                    .map_err(|_| anyhow::anyhow!("issue read state poisoned"))?;
                metrics.incr_read();
                let opens = reads.work.connection_opens;
                let result = reads.hygiene(since.as_ref());
                metrics.sqlite_open_total.fetch_add(
                    reads.work.connection_opens - opens,
                    std::sync::atomic::Ordering::Relaxed,
                );
                result.map(Some)
            })
            .await
    }

    async fn list_ready(&self, filter: ReadyFilter) -> anyhow::Result<Vec<IssueSummary>> {
        self.read(move |s| {
            let issue_types = filter
                .issue_type
                .as_deref()
                .map(|issue_type| {
                    beads_rust::model::IssueType::from_str(issue_type).map(|t| vec![t])
                })
                .transpose()?;
            let priorities = if filter.priorities.is_empty() {
                None
            } else {
                Some(
                    filter
                        .priorities
                        .iter()
                        .map(|priority| beads_rust::model::Priority(*priority))
                        .collect(),
                )
            };
            let ready_filter = beads_rust::storage::ReadyFilters {
                assignee: filter.assignee,
                labels_and: filter.labels_all,
                labels_or: filter.labels_any,
                types: issue_types,
                priorities,
                limit: Some(filter.limit.unwrap_or(20)),
                ..Default::default()
            };

            let mut issues =
                s.get_ready_issues(&ready_filter, beads_rust::storage::ReadySortPolicy::Hybrid)?;
            let ids: Vec<String> = issues.iter().map(|issue| issue.id.clone()).collect();
            let mut labels_by_id = s.get_labels_for_issues(&ids)?;
            for issue in &mut issues {
                issue.labels = labels_by_id.remove(&issue.id).unwrap_or_default();
            }

            Ok(issues.into_iter().map(br_to_pm_summary).collect())
        })
        .await
    }

    async fn list_comments(&self, issue_id: &str) -> anyhow::Result<Vec<Comment>> {
        let issue_id = issue_id.to_string();
        self.read(move |s| {
            Ok(s.get_comments(&issue_id)?
                .into_iter()
                .map(|comment| Comment {
                    id: comment.id.to_string(),
                    body: comment.body,
                    actor: comment.author,
                    created_at: comment.created_at,
                })
                .collect())
        })
        .await
    }

    async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<CommentId> {
        let issue_id = issue_id.to_string();
        let body = body.to_string();
        let actor = self.actor();
        self.write(move |s| Ok(s.add_comment(&issue_id, &actor, &body)?.id.to_string()))
            .await
    }

    async fn remove_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        let issue_id = issue_id.to_string();
        let depends_on_id = depends_on_id.to_string();
        let actor = self.actor();
        self.write(move |s| {
            s.remove_dependency(&issue_id, &depends_on_id, &actor)?;
            Ok(())
        })
        .await
    }

    async fn dep_cycles(&self) -> anyhow::Result<Vec<DependencyCycle>> {
        self.read(move |s| {
            Ok(s.detect_all_cycles()?
                .into_iter()
                .map(|issues| DependencyCycle { issues })
                .collect())
        })
        .await
    }

    async fn list_dep_hints(&self, issue_id: &str) -> anyhow::Result<Vec<ResolvedDepHint>> {
        let issue_id = issue_id.to_string();
        self.read(move |s| {
            let raw = s.get_comments(&issue_id)?;
            let mut out = Vec::new();
            for c in raw {
                if !c.body.starts_with(watermark::DEP_HINT_SENTINEL_HEADER) {
                    continue;
                }
                let Some(hint) = dep_hints::parse_dep_hint_sentinel(&c.body) else {
                    continue;
                };
                let resolved = if let Some(node_id) = hint.remote_node_id.as_deref() {
                    // Live lookup against the current external_ref index.
                    let key = format!("github:{node_id}");
                    s.find_by_external_ref(&key)?.map(|i| i.id)
                } else {
                    // Phase 1: slow-path (source_repo, remote_number) lookup
                    // is not yet wired — leave unresolved.
                    None
                };
                out.push(ResolvedDepHint {
                    hint,
                    resolved_beads_id: resolved,
                });
            }
            Ok(out)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::TempDir;

    use crate::adapter::IssueTracker;
    use crate::advanced::{BeadsAdvanced, ReadyFilter};
    use crate::beads_crate::adapter::{AdapterConfig, BeadsCrateAdapter};
    use crate::beads_crate::dependency_compat::add_dependency_with_metadata;
    use crate::types::IssueCreate;

    async fn setup_adapter() -> (TempDir, BeadsCrateAdapter) {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        (dir, adapter)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_ready_returns_unblocked_issue() {
        let (_dir, adapter) = setup_adapter().await;
        let a = adapter
            .create_issue(IssueCreate {
                title: "Task A".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let b = adapter
            .create_issue(IssueCreate {
                title: "Task B".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        adapter.add_dependency(&b, &a).await.unwrap();

        let ready = adapter
            .list_ready(ReadyFilter {
                limit: Some(50),
                ..Default::default()
            })
            .await
            .unwrap();
        let ids: HashSet<String> = ready.into_iter().map(|issue| issue.id).collect();

        assert!(
            ids.contains(&a),
            "expected unblocked issue {a}, got {ids:?}"
        );
        assert!(!ids.contains(&b), "blocked issue {b} should not be ready");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_ready_priorities_is_set_membership() {
        let (_dir, adapter) = setup_adapter().await;
        let p0 = adapter
            .create_issue(IssueCreate {
                title: "P0".into(),
                priority: Some(0),
                ..Default::default()
            })
            .await
            .unwrap();
        let p2 = adapter
            .create_issue(IssueCreate {
                title: "P2".into(),
                priority: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        let p4 = adapter
            .create_issue(IssueCreate {
                title: "P4".into(),
                priority: Some(4),
                ..Default::default()
            })
            .await
            .unwrap();

        let ready = adapter
            .list_ready(ReadyFilter {
                priorities: vec![0, 4],
                limit: Some(50),
                ..Default::default()
            })
            .await
            .unwrap();
        let ids: HashSet<String> = ready.into_iter().map(|issue| issue.id).collect();

        assert!(ids.contains(&p0), "priority 0 issue missing: {ids:?}");
        assert!(ids.contains(&p4), "priority 4 issue missing: {ids:?}");
        assert!(
            !ids.contains(&p2),
            "priority 2 issue should be filtered out"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_ready_empty_priorities_returns_all_priority_levels() {
        let (_dir, adapter) = setup_adapter().await;
        let p0 = adapter
            .create_issue(IssueCreate {
                title: "P0".into(),
                priority: Some(0),
                ..Default::default()
            })
            .await
            .unwrap();
        let p2 = adapter
            .create_issue(IssueCreate {
                title: "P2".into(),
                priority: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        let p4 = adapter
            .create_issue(IssueCreate {
                title: "P4".into(),
                priority: Some(4),
                ..Default::default()
            })
            .await
            .unwrap();

        let ready = adapter
            .list_ready(ReadyFilter {
                priorities: vec![],
                limit: Some(50),
                ..Default::default()
            })
            .await
            .unwrap();
        let ids: HashSet<String> = ready.into_iter().map(|issue| issue.id).collect();

        assert!(ids.contains(&p0), "priority 0 issue missing: {ids:?}");
        assert!(ids.contains(&p2), "priority 2 issue missing: {ids:?}");
        assert!(ids.contains(&p4), "priority 4 issue missing: {ids:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_comments_returns_existing_comment() {
        let (_dir, adapter) = setup_adapter().await;
        let issue_id = adapter
            .create_issue(IssueCreate {
                title: "Commented".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let issue_for_seed = issue_id.clone();
        adapter
            .write(move |s| {
                s.add_comment(&issue_for_seed, "test", "seeded comment")?;
                Ok(())
            })
            .await
            .unwrap();

        let comments = adapter.list_comments(&issue_id).await.unwrap();

        assert!(
            comments
                .iter()
                .any(|comment| comment.body == "seeded comment" && comment.actor == "test"),
            "expected seeded comment, got {comments:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_comment_persists_comment() {
        let (_dir, adapter) = setup_adapter().await;
        let issue_id = adapter
            .create_issue(IssueCreate {
                title: "New comment".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let comment_id = adapter
            .add_comment(&issue_id, "advanced comment")
            .await
            .unwrap();

        assert!(!comment_id.is_empty());

        // True trait round-trip: add via BeadsAdvanced::add_comment, read via
        // BeadsAdvanced::list_comments — verifies the public surface end-to-end
        // (the previous version went around list_comments via raw get_comments).
        let comments = adapter.list_comments(&issue_id).await.unwrap();
        assert!(
            comments.iter().any(|comment| comment.id == comment_id
                && comment.body == "advanced comment"
                && comment.actor == "spur"),
            "expected advanced comment id {comment_id} via list_comments, got {comments:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn actor_threads_through_write_methods() {
        let dir = TempDir::new().unwrap();
        let config = AdapterConfig {
            actor: "test-actor".to_string(),
            ..AdapterConfig::default()
        };
        let adapter = BeadsCrateAdapter::open(dir.path(), config).await.unwrap();
        let id = adapter
            .create_issue(IssueCreate {
                title: "actor threading test".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let issue_for_check = id.clone();
        let created_by = adapter
            .read(move |s| {
                Ok(s.get_issue(&issue_for_check)?
                    .and_then(|issue| issue.created_by))
            })
            .await
            .unwrap();
        assert_eq!(created_by.as_deref(), Some("test-actor"));

        let _comment_id = adapter.add_comment(&id, "test body").await.unwrap();
        let comments = adapter.list_comments(&id).await.unwrap();
        assert!(comments.iter().any(|c| c.actor == "test-actor"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_dependency_round_trips() {
        let (_dir, adapter) = setup_adapter().await;
        let parent = adapter
            .create_issue(IssueCreate {
                title: "Parent".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let child = adapter
            .create_issue(IssueCreate {
                title: "Child".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        adapter.add_dependency(&child, &parent).await.unwrap();

        adapter.remove_dependency(&child, &parent).await.unwrap();

        let child_for_check = child.clone();
        let deps = adapter
            .read(move |s| Ok(s.get_dependencies(&child_for_check)?))
            .await
            .unwrap();
        assert!(
            !deps.contains(&parent),
            "dependency should have been removed, got {deps:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dep_cycles_returns_storage_detected_cycles() {
        let (_dir, adapter) = setup_adapter().await;
        let a = adapter
            .create_issue(IssueCreate {
                title: "Cycle A".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let b = adapter
            .create_issue(IssueCreate {
                title: "Cycle B".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let a_for_seed = a.clone();
        let b_for_seed = b.clone();
        adapter
            .write(move |s| {
                add_dependency_with_metadata(s, &a_for_seed, &b_for_seed, "related", "test", None)?;
                add_dependency_with_metadata(s, &b_for_seed, &a_for_seed, "related", "test", None)?;
                Ok(())
            })
            .await
            .unwrap();

        let cycles = adapter.dep_cycles().await.unwrap();

        assert!(
            cycles
                .iter()
                .any(|cycle| cycle.issues.contains(&a) && cycle.issues.contains(&b)),
            "expected cycle containing {a} and {b}, got {cycles:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn plan_id_label_for_epic_returns_plan_label_only_for_epics() {
        let (_dir, adapter) = setup_adapter().await;
        let epic = adapter
            .create_issue(IssueCreate {
                title: "Epic".into(),
                issue_type: Some("epic".into()),
                labels: vec!["other".into(), "spur:plan-id:P1".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        let task = adapter
            .create_issue(IssueCreate {
                title: "Task".into(),
                labels: vec!["spur:plan-id:P2".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        let epic_label = adapter.plan_id_label_for_epic(&epic).await.unwrap();
        let task_label = adapter.plan_id_label_for_epic(&task).await.unwrap();

        assert_eq!(epic_label.as_deref(), Some("spur:plan-id:P1"));
        assert_eq!(task_label, None);
    }
}
