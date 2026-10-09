use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

use crate::adapter::IssueTracker;
use crate::beads_crate::{AdapterConfig, BeadsCrateAdapter};
use crate::types::{IssueFilter, IssueSummary, PmSource};

// Frozen adapter at 083241051. Keep independent of the new SQL builder.
async fn legacy_list(
    adapter: &BeadsCrateAdapter,
    filter: IssueFilter,
) -> anyhow::Result<Vec<IssueSummary>> {
    adapter
        .read(move |s| {
            let mut br_filters = beads_rust::storage::sqlite::ListFilters::default();

            if !filter.labels.is_empty() {
                br_filters.labels = Some(filter.labels.clone());
            }
            if let Some(status) = filter.status.as_deref() {
                let parsed = beads_rust::model::Status::from_str(status)
                    .unwrap_or(beads_rust::model::Status::Open);
                br_filters.statuses = Some(vec![parsed]);
            }
            if let Some(itype) = filter.issue_type.as_deref() {
                let parsed = beads_rust::model::IssueType::from_str(itype)
                    .unwrap_or(beads_rust::model::IssueType::Task);
                br_filters.types = Some(vec![parsed]);
            }
            br_filters.assignee = filter.assignee.clone();
            if let Some(min) = filter.priority_min {
                let max = filter.priority_max.unwrap_or(4);
                let priorities: Vec<beads_rust::model::Priority> =
                    (min..=max).map(beads_rust::model::Priority).collect();
                br_filters.priorities = Some(priorities);
            } else if let Some(max) = filter.priority_max {
                let priorities: Vec<beads_rust::model::Priority> =
                    (0..=max).map(beads_rust::model::Priority).collect();
                br_filters.priorities = Some(priorities);
            }
            br_filters.title_contains = filter.text_search.clone();
            br_filters.include_closed = filter.include_closed || filter.status.is_some();
            let offset = filter.offset.unwrap_or(0);
            br_filters.limit = filter.limit.map(|limit| limit.saturating_add(offset));
            if let Some(since) = filter.since {
                br_filters.updated_after = Some(since);
            }

            let mut issues = s.list_issues(&br_filters)?;
            if !br_filters
                .statuses
                .as_ref()
                .is_some_and(|statuses| statuses.contains(&beads_rust::model::Status::Tombstone))
            {
                issues.retain(|issue| issue.status != beads_rust::model::Status::Tombstone);
            }
            let ids = issues.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
            let mut labels = s.get_labels_for_issues(&ids)?;
            let summaries = issues.into_iter().skip(offset).map(|issue| IssueSummary {
                labels: labels.remove(&issue.id).unwrap_or_default(),
                url: format!("beads://{}", issue.id),
                id: issue.id,
                source: PmSource::Beads,
                title: issue.title,
                description: issue.description,
                status: issue.status.as_str().to_owned(),
                priority: Some(issue.priority.0),
                issue_type: Some(issue.issue_type.as_str().to_owned()),
                assignee: issue.assignee,
            });
            Ok(match filter.limit {
                Some(limit) => summaries.take(limit).collect(),
                None => summaries.collect(),
            })
        })
        .await
}

fn insert(
    conn: &Connection,
    id: &str,
    status: &str,
    priority: i32,
    kind: &str,
    template: bool,
    created: &str,
) {
    conn.execute(
        "INSERT INTO issues (id, title, description, status, priority, issue_type, assignee, is_template, created_at, updated_at, closed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, CASE WHEN ?4 = 'closed' THEN ?9 END)",
        rusqlite::params![id, format!("title {id} 50%_\\ '"), format!("loop description {id}"), status, priority, kind, "alice", template, created],
    ).unwrap();
}

async fn fixture() -> (TempDir, BeadsCrateAdapter, Connection) {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let conn = Connection::open(dir.path().join("beads.db")).unwrap();
    let statuses = [
        "open",
        "in_progress",
        "blocked",
        "closed",
        "deferred",
        "tombstone",
        "pinned",
        "custom",
        "OPEN",
    ];
    for (i, status) in statuses.iter().enumerate() {
        for (j, kind) in ["task", "bug", "feature", "epic", "chore", "CUSTOM"]
            .iter()
            .enumerate()
        {
            let id = format!("bd-{i}-{j}");
            insert(
                &conn,
                &id,
                status,
                j as i32 % 5,
                kind,
                false,
                if i % 2 == 0 {
                    "2026-10-08T00:00:00+00:00"
                } else {
                    "2026-10-09T00:00:00+00:00"
                },
            );
            for label in ["broad", if j % 2 == 0 { "even" } else { "odd" }] {
                conn.execute(
                    "INSERT INTO labels (issue_id, label) VALUES (?1, ?2)",
                    [&id, label],
                )
                .unwrap();
            }
        }
    }
    insert(
        &conn,
        "bd-template",
        "open",
        0,
        "task",
        true,
        "2026-10-09T00:00:00+00:00",
    );
    conn.execute(
        "UPDATE issues SET description = '', assignee = '' WHERE id = 'bd-0-0'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE issues SET assignee = NULL, is_template = NULL WHERE id = 'bd-0-1'",
        [],
    )
    .unwrap();
    (dir, adapter, conn)
}

#[tokio::test]
async fn summary_projection_does_not_decode_unneeded_metadata() {
    let (_dir, adapter, conn) = fixture().await;
    let expected = legacy_list(&adapter, IssueFilter::default()).await.unwrap();
    conn.execute("UPDATE issues SET due_at = 'not-a-date'", [])
        .unwrap();
    assert!(
        legacy_list(&adapter, IssueFilter::default()).await.is_err(),
        "fixture must exercise upstream full Issue decoding"
    );
    let result = adapter.list_issues(IssueFilter::default()).await;
    assert!(
        result.is_ok(),
        "summary must not decode unneeded due_at: {result:?}"
    );
    assert_eq!(
        serde_json::to_value(result.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
async fn summary_projection_matches_legacy_filter_and_page_matrix() {
    let (_dir, adapter, _conn) = fixture().await;
    let mut filters = vec![IssueFilter::default()];
    for status in [
        "open",
        "in_progress",
        "inprogress",
        "OPEN",
        "blocked",
        "closed",
        "deferred",
        "tombstone",
        "pinned",
        "invalid",
        "",
    ] {
        filters.push(IssueFilter {
            status: Some(status.into()),
            ..Default::default()
        });
    }
    for kind in [
        "task", "BUG", "feature", "epic", "chore", "custom", "invalid", "",
    ] {
        filters.push(IssueFilter {
            issue_type: Some(kind.into()),
            ..Default::default()
        });
    }
    for labels in [
        vec!["broad"],
        vec!["broad", "even"],
        vec!["even", "odd"],
        vec!["even", "even"],
        vec!["missing"],
    ] {
        filters.push(IssueFilter {
            labels: labels.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        });
    }
    for (min, max) in [
        (Some(1), Some(3)),
        (Some(4), Some(1)),
        (Some(5), None),
        (None, Some(-1)),
        (Some(-1), Some(0)),
        (None, Some(2)),
        (Some(2), None),
    ] {
        filters.push(IssueFilter {
            priority_min: min,
            priority_max: max,
            ..Default::default()
        });
    }
    for text in ["50%_\\", "%", "_", "\\", "'", "TITLE", "", "absent"] {
        filters.push(IssueFilter {
            text_search: Some(text.into()),
            ..Default::default()
        });
    }
    for assignee in ["alice", "", "missing"] {
        filters.push(IssueFilter {
            assignee: Some(assignee.into()),
            ..Default::default()
        });
    }
    filters.push(IssueFilter {
        since: Some(Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0).unwrap()),
        ..Default::default()
    });
    filters.push(IssueFilter {
        status: Some("open".into()),
        issue_type: Some("task".into()),
        labels: vec!["broad".into(), "even".into()],
        priority_max: Some(3),
        text_search: Some("title".into()),
        ..Default::default()
    });
    for base in filters {
        for include_closed in [false, true] {
            for limit in [None, Some(0), Some(1), Some(7), Some(100)] {
                for offset in [None, Some(0), Some(2), Some(100)] {
                    let filter = IssueFilter {
                        include_closed,
                        limit,
                        offset,
                        ..base.clone()
                    };
                    let expected = legacy_list(&adapter, filter.clone()).await.unwrap();
                    let actual = adapter.list_issues(filter.clone()).await.unwrap();
                    assert_eq!(
                        serde_json::to_value(actual).unwrap(),
                        serde_json::to_value(expected).unwrap(),
                        "filter: {filter:?}"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn summary_projection_observes_external_edits_and_reuses_connections() {
    let (_dir, adapter, conn) = fixture().await;
    let filter = IssueFilter {
        status: Some("open".into()),
        labels: vec!["even".into()],
        ..Default::default()
    };
    adapter.list_issues(filter.clone()).await.unwrap();
    let opens = adapter.metrics().sqlite_open_total.load(Ordering::Relaxed);
    for description in ["external one", "external two"] {
        conn.execute(
            "UPDATE issues SET description = ?1 WHERE id = 'bd-0-2'",
            [description],
        )
        .unwrap();
        let rows = adapter.list_issues(filter.clone()).await.unwrap();
        assert_eq!(
            rows.iter()
                .find(|i| i.id == "bd-0-2")
                .unwrap()
                .description
                .as_deref(),
            Some(description)
        );
    }
    conn.execute(
        "DELETE FROM labels WHERE issue_id = 'bd-0-2' AND label = 'even'",
        [],
    )
    .unwrap();
    assert!(!adapter
        .list_issues(filter.clone())
        .await
        .unwrap()
        .iter()
        .any(|i| i.id == "bd-0-2"));
    conn.execute(
        "UPDATE issues SET status = 'closed', closed_at = updated_at WHERE id = 'bd-0-4'",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM issues WHERE id = 'bd-0-0'", [])
        .unwrap();
    let rows = adapter.list_issues(filter).await.unwrap();
    assert!(!rows
        .iter()
        .any(|i| ["bd-0-0", "bd-0-4"].contains(&i.id.as_str())));
    assert_eq!(
        adapter.metrics().sqlite_open_total.load(Ordering::Relaxed),
        opens,
        "repeated list requests must not open connections"
    );
}

#[tokio::test]
async fn summary_projection_preserves_projected_and_query_errors() {
    // Invalid metadata outside the projection is intentionally excluded from
    // this contract; all seven projected columns keep upstream errors.
    for column in [
        "id",
        "title",
        "description",
        "status",
        "priority",
        "issue_type",
        "assignee",
    ] {
        let (_dir, adapter, conn) = fixture().await;
        conn.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute(
            &format!("UPDATE issues SET {column} = x'ff' WHERE id = 'bd-0-0'"),
            [],
        )
        .unwrap();
        for limit in [None, Some(0)] {
            let filter = IssueFilter {
                limit,
                ..Default::default()
            };
            let expected = legacy_list(&adapter, filter.clone()).await.unwrap_err();
            let actual = adapter.list_issues(filter).await.unwrap_err();
            assert_eq!(
                actual.to_string(),
                expected.to_string(),
                "column {column}, limit {limit:?}"
            );
            assert!(actual
                .downcast_ref::<beads_rust::error::BeadsError>()
                .is_some());
        }
    }
    let (_dir, adapter, conn) = fixture().await;
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    conn.execute(
        "UPDATE issues SET priority = 2147483648 WHERE id = 'bd-0-0'",
        [],
    )
    .unwrap();
    let expected = legacy_list(&adapter, IssueFilter::default())
        .await
        .unwrap_err();
    assert_eq!(
        adapter
            .list_issues(IssueFilter::default())
            .await
            .unwrap_err()
            .to_string(),
        expected.to_string()
    );
    conn.execute("UPDATE issues SET priority = 0 WHERE id = 'bd-0-0'", [])
        .unwrap();
    let filter = IssueFilter {
        limit: Some(usize::MAX),
        offset: Some(2),
        ..Default::default()
    };
    let expected = legacy_list(&adapter, filter.clone()).await.unwrap_err();
    assert_eq!(
        adapter.list_issues(filter).await.unwrap_err().to_string(),
        expected.to_string()
    );
    conn.execute("DROP TABLE labels", []).unwrap();
    for limit in [None, Some(0)] {
        let filter = IssueFilter {
            limit,
            ..Default::default()
        };
        let expected = legacy_list(&adapter, filter.clone()).await.unwrap_err();
        assert_eq!(
            adapter.list_issues(filter).await.unwrap_err().to_string(),
            expected.to_string()
        );
    }
}

#[tokio::test]
async fn summary_projection_retains_batched_sorted_labels() {
    let (dir, adapter, conn) = fixture().await;
    // Cross the pinned upstream label batch size of 900.
    for i in 0..901 {
        let id = format!("bd-batch-{i:04}");
        insert(
            &conn,
            &id,
            "open",
            2,
            "task",
            false,
            "2026-10-09T00:00:00+00:00",
        );
        for label in ["z-last", "a-first"] {
            conn.execute(
                "INSERT INTO labels (issue_id, label) VALUES (?1, ?2)",
                [&id, label],
            )
            .unwrap();
        }
    }
    let filter = IssueFilter {
        text_search: Some("bd-batch-".into()),
        ..Default::default()
    };
    let expected = legacy_list(&adapter, filter.clone()).await.unwrap();
    assert_eq!(expected.len(), 901);
    let actual = adapter.list_issues(filter.clone()).await.unwrap();
    assert!(actual.iter().all(|i| i.labels == ["a-first", "z-last"]));
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    drop(adapter);
    let reopened = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(reopened.list_issues(filter).await.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
