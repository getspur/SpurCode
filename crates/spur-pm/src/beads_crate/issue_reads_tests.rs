//! End-to-end work contracts use the production tracing counters, not mocks.
use crate::adapter::IssueTracker;
use crate::beads_crate::beads_db::trace_tests::Capture;
use crate::beads_crate::{AdapterConfig, BeadsCrateAdapter};
use crate::types::{IssueFilter, IssueSummary};
use crate::BeadsAdvanced;
use rusqlite::Connection;
use tempfile::TempDir;
use tracing::instrument::WithSubscriber;

async fn fixture(size: usize) -> (TempDir, BeadsCrateAdapter, Connection) {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let conn = Connection::open(dir.path().join("beads.db")).unwrap();
    conn.execute_batch("BEGIN").unwrap();
    for i in 0..size {
        conn.execute("INSERT INTO issues(id,title,description,status,priority,issue_type,created_at,updated_at) VALUES (?1,?1,'before',?2,?3,'task',?4,'2026-10-09T00:00:00Z')",
            rusqlite::params![format!("bd-{i:05}"), if i%2==0 {"open"} else {"blocked"}, i%5, format!("2026-10-09T00:00:{:02}Z", i%60)]).unwrap();
        conn.execute(
            "INSERT INTO labels(issue_id,label) VALUES (?1,'hot')",
            [format!("bd-{i:05}")],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    (dir, adapter, conn)
}
fn filter() -> IssueFilter {
    IssueFilter {
        status: Some("open".into()),
        labels: vec!["hot".into()],
        ..Default::default()
    }
}
async fn measured(
    adapter: &BeadsCrateAdapter,
    filter: IssueFilter,
) -> (Vec<IssueSummary>, u64, u64) {
    let capture = Capture::default();
    let rows = adapter
        .list_issues(filter)
        .with_subscriber(capture.dispatch())
        .await
        .unwrap();
    let events = capture.events();
    let sum = |name: &str| {
        events
            .iter()
            .filter(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\""))
            .map(|e| {
                e.fields
                    .0
                    .get(name)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
            })
            .sum()
    };
    (rows, sum("sql_rows"), sum("label_rows"))
}
fn same(a: &[IssueSummary], b: &[IssueSummary]) {
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
}
fn direct(conn: &Connection, filter: &IssueFilter) -> Vec<IssueSummary> {
    super::summary_query::list_summaries(conn, filter).unwrap()
}

#[tokio::test]
async fn issue_reads_unchanged_work_is_zero_at_different_sizes() {
    for size in [12, 1200] {
        let (_dir, adapter, _conn) = fixture(size).await;
        let (before, _, _) = measured(&adapter, filter()).await;
        let (after, issues, labels) = measured(&adapter, filter()).await;
        same(&before, &after);
        assert_eq!(
            (issues, labels),
            (0, 0),
            "unchanged hot list must load zero rows at size {size}"
        );
        let work = adapter.issue_read_work();
        assert_eq!(
            (
                work.full_builds,
                work.reuses,
                work.issue_rows,
                work.label_rows
            ),
            (1, 1, size as u64 / 2, size as u64 / 2)
        );
        assert_eq!(
            work.connection_opens, 1,
            "hot reads reuse the feed connection"
        );
        eprintln!("S5 unchanged size={size} work={work:?}");
    }
}

#[tokio::test]
async fn issue_reads_relevant_delta_loads_only_affected_payload() {
    for size in [12, 1200] {
        let (_dir, adapter, conn) = fixture(size).await;
        measured(&adapter, filter()).await;
        conn.execute(
            "UPDATE issues SET description='after' WHERE id='bd-00000'",
            [],
        )
        .unwrap();
        let (rows, issues, labels) = measured(&adapter, filter()).await;
        same(&rows, &direct(&conn, &filter()));
        assert_eq!(
            (issues, labels),
            (1, 1),
            "delta work must not scale with fixture size {size}"
        );
        let work = adapter.issue_read_work();
        assert_eq!(
            (work.full_builds, work.delta_refreshes, work.candidate_ids),
            (1, 1, 1)
        );
        eprintln!("S5 delta size={size} work={work:?}");
    }
}

#[tokio::test]
async fn issue_reads_unrelated_edit_and_adapter_comment_load_no_payload() {
    let (_dir, adapter, conn) = fixture(12).await;
    let (before, _, _) = measured(&adapter, filter()).await;
    conn.execute(
        "UPDATE issues SET description='unrelated' WHERE id='bd-00001'",
        [],
    )
    .unwrap();
    let (after, issues, labels) = measured(&adapter, filter()).await;
    same(&before, &after);
    assert_eq!(
        (issues, labels),
        (0, 0),
        "nonmember edits must not decode payload"
    );
    adapter
        .add_comment("bd-00000", "ordinary comment")
        .await
        .unwrap();
    let (after, issues, labels) = measured(&adapter, filter()).await;
    same(&before, &after);
    assert_eq!(
        (issues, labels),
        (0, 0),
        "adapter comments update timestamps but not since=None summaries"
    );
}

#[tokio::test]
async fn issue_reads_old_and_new_membership_and_order_match_sql() {
    let (_dir, adapter, conn) = fixture(12).await;
    // External SQL writers may disable FKs when replacing IDs.
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    measured(&adapter, filter()).await;
    for sql in [
        "UPDATE issues SET status='open' WHERE id='bd-00001'",
        "UPDATE issues SET priority=0,created_at='2026-10-10T00:00:00Z' WHERE id='bd-00002'",
        "UPDATE issues SET status='closed',closed_at=updated_at WHERE id='bd-00000'",
        "DELETE FROM labels WHERE issue_id='bd-00002'",
        "DELETE FROM issues WHERE id='bd-00004'",
        "UPDATE issues SET status='tombstone' WHERE id='bd-00006'",
        "INSERT INTO labels(issue_id,label) VALUES ('bd-00002','hot')",
        "UPDATE issues SET id='bd-renamed' WHERE id='bd-00008'",
    ] {
        conn.execute(sql, []).unwrap();
        let (rows, _, _) = measured(&adapter, filter()).await;
        same(&rows, &direct(&conn, &filter()));
    }
}

#[tokio::test]
async fn issue_reads_pages_and_since_keep_sql_boundaries() {
    let (_dir, adapter, conn) = fixture(12).await;
    for limit in [None, Some(0), Some(1), Some(4)] {
        for offset in [None, Some(0), Some(2), Some(20)] {
            let f = IssueFilter {
                limit,
                offset,
                include_closed: true,
                ..Default::default()
            };
            measured(&adapter, f.clone()).await;
            conn.execute("UPDATE issues SET priority=0 WHERE id='bd-00011'", [])
                .unwrap();
            let (rows, _, _) = measured(&adapter, f.clone()).await;
            same(&rows, &direct(&conn, &f));
        }
    }
    let f = IssueFilter {
        since: Some(chrono::Utc::now()),
        ..filter()
    };
    measured(&adapter, f.clone()).await;
    adapter
        .add_comment("bd-00000", "since timestamp changes")
        .await
        .unwrap();
    let (rows, _, _) = measured(&adapter, f.clone()).await;
    same(&rows, &direct(&conn, &f));
}

#[tokio::test]
async fn issue_reads_identical_concurrent_requests_coalesce() {
    let (_dir, adapter, conn) = fixture(12).await;
    measured(&adapter, filter()).await;
    conn.execute(
        "UPDATE issues SET description='after' WHERE id='bd-00000'",
        [],
    )
    .unwrap();
    let (a, b, c, d) = tokio::join!(
        measured(&adapter, filter()),
        measured(&adapter, filter()),
        measured(&adapter, filter()),
        measured(&adapter, filter())
    );
    for rows in [&a.0, &b.0, &c.0, &d.0] {
        same(rows, &direct(&conn, &filter()));
    }
    assert_eq!(
        (a.1 + b.1 + c.1 + d.1, a.2 + b.2 + c.2 + d.2),
        (1, 1),
        "one refresh serves all four readers"
    );
}

#[tokio::test]
async fn issue_reads_gap_and_schema_change_reseed() {
    let (_dir, adapter, conn) = fixture(12).await;
    measured(&adapter, filter()).await;
    for sql in [
        "DELETE FROM spur_issue_changes",
        "CREATE TABLE unrelated_schema_change(id INTEGER)",
    ] {
        // First-read schema repair may legitimately clear the old journal.
        // Create an unseen edit before destroying coverage, not an empty DELETE.
        conn.execute(
            "UPDATE issues SET description=?1 WHERE id='bd-00000'",
            [sql],
        )
        .unwrap();
        conn.execute(sql, []).unwrap();
        let (rows, issues, _) = measured(&adapter, filter()).await;
        same(&rows, &direct(&conn, &filter()));
        assert_eq!(issues, rows.len() as u64);
        let (_, issues, labels) = measured(&adapter, filter()).await;
        assert_eq!((issues, labels), (0, 0));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn issue_reads_database_replacement_rebinds_cached_and_direct_paths() {
    let (dir, adapter, conn) = fixture(12).await;
    let (other, other_adapter, other_conn) = fixture(4).await;
    other_conn
        .execute("UPDATE issues SET description='replacement'", [])
        .unwrap();
    // A standalone SQLite backup avoids swapping active WAL sidecars.
    let replacement = other.path().join("replacement.db");
    other_conn
        .execute("VACUUM INTO ?1", [replacement.to_str().unwrap()])
        .unwrap();
    measured(&adapter, filter()).await;
    drop(other_conn);
    drop(other_adapter);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);
    std::fs::rename(replacement, dir.path().join("beads.db")).unwrap();
    for f in [
        filter(),
        IssueFilter {
            limit: Some(2),
            ..filter()
        },
    ] {
        let (rows, _, _) = measured(&adapter, f).await;
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.description.as_deref() == Some("replacement")),
            "must reopen the new inode"
        );
    }
}

#[tokio::test]
async fn issue_reads_commit_during_load_leaves_snapshot_dirty() {
    use crate::beads_crate::beads_db::trace_tests::Fields;
    use std::sync::Mutex;
    use tracing_subscriber::prelude::*;
    struct CommitDuringLoad(Mutex<Option<std::path::PathBuf>>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CommitDuringLoad {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            if fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\"") {
                if let Some(path) = self.0.lock().unwrap().take() {
                    let conn = Connection::open(path).unwrap();
                    conn.execute(
                        "UPDATE issues SET description='raced' WHERE id='bd-00000'",
                        [],
                    )
                    .unwrap();
                }
            }
        }
    }
    let (dir, adapter, _conn) = fixture(12).await;
    let capture = Capture::default();
    let dispatch = tracing::Dispatch::new(
        tracing_subscriber::Registry::default()
            .with(capture.clone())
            .with(CommitDuringLoad(Mutex::new(Some(
                dir.path().join("beads.db"),
            )))),
    );
    let snapshot = adapter
        .list_issues(filter())
        .with_subscriber(dispatch)
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .iter()
            .find(|r| r.id == "bd-00000")
            .unwrap()
            .description
            .as_deref(),
        Some("before")
    );
    assert!(
        capture
            .events()
            .iter()
            .any(|e| e.fields.0.get("races").map(String::as_str) == Some("1")),
        "commit during load must be detected and leave the published snapshot dirty"
    );
    let (rows, issues, labels) = measured(&adapter, filter()).await;
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "bd-00000")
            .unwrap()
            .description
            .as_deref(),
        Some("raced")
    );
    assert_eq!((issues, labels), (1, 1));
    let (_, issues, labels) = measured(&adapter, filter()).await;
    assert_eq!((issues, labels), (0, 0));
}

#[tokio::test]
async fn issue_reads_retention_obeys_database_budget_and_reader_width() {
    let (_dir, adapter, conn) = fixture(12).await;
    for priority in 0..8 {
        let capture = Capture::default();
        adapter
            .list_issues(IssueFilter {
                priority_min: Some(priority),
                ..filter()
            })
            .with_subscriber(capture.dispatch())
            .await
            .unwrap();
        let events = capture.events();
        let work = events
            .iter()
            .find(|e| e.fields.0.contains_key("retained_bytes"))
            .expect("production work counters must expose retention and its derived budget");
        let n = |key: &str| work.fields.0[key].parse::<usize>().unwrap();
        assert!(n("retained_bytes") <= n("budget_bytes"));
        assert!(n("subscriptions") <= super::beads_db::DEFAULT_READER_THREADS);
    }
    // Force a single summary larger than the current pager budget; it must
    // remain directly queryable without being retained by the subscription.
    let cache_size: i64 = conn
        .query_row("PRAGMA cache_size", [], |r| r.get(0))
        .unwrap();
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let budget = if cache_size < 0 {
        -cache_size * 1024
    } else {
        cache_size * page_size
    };
    conn.execute(
        "UPDATE issues SET description=?1 WHERE id='bd-00000'",
        ["x".repeat(budget as usize + 1)],
    )
    .unwrap();
    let (_, first, _) = measured(&adapter, filter()).await;
    let (_, second, _) = measured(&adapter, filter()).await;
    assert!(
        first > 0 && second > 0,
        "oversized result must take direct SQL without unbounded retention"
    );
}

#[tokio::test]
async fn issue_reads_ordinary_adapter_comment_is_summary_unrelated() {
    let (_dir, adapter, _conn) = fixture(12).await;
    let (before, _, _) = measured(&adapter, filter()).await;
    adapter
        .add_comment("bd-00000", "ordinary comment")
        .await
        .unwrap();
    let (after, issues, labels) = measured(&adapter, filter()).await;
    same(&before, &after);
    assert_eq!((issues, labels), (0, 0));
}

// This event runs after the load transaction has committed and before the
// validation transaction starts. Mutating from the query timing event would
// instead exercise replacement during the load, a different feed error path.
fn before_validation(action: impl FnOnce() + Send + 'static) -> tracing::Dispatch {
    use crate::beads_crate::beads_db::trace_tests::Fields;
    use std::sync::Mutex;
    use tracing_subscriber::prelude::*;
    struct BeforeValidation(Mutex<Option<Box<dyn FnOnce() + Send>>>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for BeforeValidation {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            if fields.0.get("site").map(String::as_str) == Some("\"beads_issue_reads_validate\"") {
                if let Some(action) = self.0.lock().unwrap().take() {
                    action();
                }
            }
        }
    }
    tracing::Dispatch::new(
        tracing_subscriber::Registry::default()
            .with(BeforeValidation(Mutex::new(Some(Box::new(action))))),
    )
}

async fn validation_reset_discards_and_retries(reset_sql: &'static str) {
    let (dir, adapter, conn) = fixture(12).await;
    measured(&adapter, filter()).await;
    conn.execute(
        "UPDATE issues SET description='loaded' WHERE id='bd-00000'",
        [],
    )
    .unwrap();
    let before = adapter.issue_read_work();
    let path = dir.path().join("beads.db");
    let result = adapter
        .list_issues(filter())
        .with_subscriber(before_validation(move || {
            let conn = Connection::open(path).unwrap();
            conn.execute(
                "UPDATE issues SET description='after reset' WHERE id='bd-00000'",
                [],
            )
            .unwrap();
            conn.execute_batch(reset_sql).unwrap();
        }))
        .await;
    assert!(
        result.is_err(),
        "validation reset must discard the old candidate: {reset_sql}"
    );
    let after = adapter.issue_read_work();
    assert_eq!(
        after.subscriptions, 0,
        "discarded candidate must not be retained"
    );
    assert_eq!(
        (
            after.issue_rows - before.issue_rows,
            after.label_rows - before.label_rows
        ),
        (1, 1)
    );
    assert_eq!(
        after.delta_refreshes - before.delta_refreshes,
        1,
        "discarded work is still counted"
    );
    assert_eq!(
        after.races, before.races,
        "reset is not an ordinary revision race"
    );
    let (rows, issues, labels) = measured(&adapter, filter()).await;
    same(&rows, &direct(&conn, &filter()));
    assert_eq!(
        (issues, labels),
        (6, 6),
        "retry must reseed, not reuse discarded membership"
    );
    let (_, issues, labels) = measured(&adapter, filter()).await;
    assert_eq!((issues, labels), (0, 0));
}

#[tokio::test]
async fn issue_reads_validation_generation_reset_discards_and_retries() {
    validation_reset_discards_and_retries(
        "UPDATE spur_issue_clock SET generation='4674f081-5126-434d-acf7-5e5b602ae805'",
    )
    .await;
}

#[tokio::test]
async fn issue_reads_validation_gap_discards_and_retries() {
    validation_reset_discards_and_retries("DELETE FROM spur_issue_changes").await;
}

#[tokio::test]
async fn issue_reads_validation_schema_reset_discards_and_retries() {
    validation_reset_discards_and_retries("CREATE TABLE validation_schema_change(id INTEGER)")
        .await;
}

#[cfg(unix)]
#[tokio::test]
async fn issue_reads_validation_replacement_discards_and_rebinds() {
    let (dir, adapter, conn) = fixture(12).await;
    measured(&adapter, filter()).await;
    let replacement = dir.path().join("replacement.db");
    // Clone the current DB so its tracking generation survives replacement.
    conn.execute("VACUUM INTO ?1", [replacement.to_str().unwrap()])
        .unwrap();
    let replacement_conn = Connection::open(&replacement).unwrap();
    replacement_conn
        .execute("UPDATE issues SET description='replacement'", [])
        .unwrap();
    drop(replacement_conn);
    conn.execute(
        "UPDATE issues SET description='loaded' WHERE id='bd-00000'",
        [],
    )
    .unwrap();
    let before = adapter.issue_read_work();
    let path = dir.path().join("beads.db");
    let result = adapter
        .list_issues(filter())
        .with_subscriber(before_validation(move || {
            // The load transaction has finished, so no active snapshot pins WAL.
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            drop(conn);
            std::fs::rename(replacement, path).unwrap();
        }))
        .await;
    assert!(
        result.is_err(),
        "known replacement must not return an old-inode candidate"
    );
    let after = adapter.issue_read_work();
    assert_eq!(after.subscriptions, 0);
    assert_eq!(
        (
            after.issue_rows - before.issue_rows,
            after.label_rows - before.label_rows
        ),
        (1, 1)
    );
    assert_eq!(
        after.connection_opens,
        before.connection_opens + 1,
        "validation rebinds the persistent connection"
    );
    assert_eq!(after.races, before.races);
    let (rows, issues, labels) = measured(&adapter, filter()).await;
    assert_eq!((issues, labels), (6, 6));
    assert!(rows
        .iter()
        .all(|r| r.description.as_deref() == Some("replacement")));
    let (_, issues, labels) = measured(&adapter, filter()).await;
    assert_eq!((issues, labels), (0, 0));
}

#[tokio::test]
async fn issue_reads_validation_error_propagates_and_retry_reseeds() {
    for warm in [false, true] {
        let (dir, adapter, conn) = fixture(12).await;
        if warm {
            measured(&adapter, filter()).await;
            conn.execute(
                "UPDATE issues SET description='loaded' WHERE id='bd-00000'",
                [],
            )
            .unwrap();
        }
        let before = adapter.issue_read_work();
        let path = dir.path().join("beads.db");
        let result = adapter
            .read_summaries(filter())
            .with_subscriber(before_validation(move || {
                Connection::open(path)
                    .unwrap()
                    .execute_batch("ALTER TABLE labels RENAME TO validation_labels")
                    .unwrap();
            }))
            .await;
        let error =
            result.expect_err("validation failure must propagate instead of returning loaded rows");
        assert!(
            error
                .to_string()
                .contains("missing issue-feed source table labels"),
            "{error:#}"
        );
        let after = adapter.issue_read_work();
        assert_eq!(after.subscriptions, 0);
        let loaded = if warm { 1 } else { 6 };
        assert_eq!(
            (
                after.issue_rows - before.issue_rows,
                after.label_rows - before.label_rows
            ),
            (loaded, loaded)
        );
        assert_eq!(after.races, before.races);
        conn.execute_batch("ALTER TABLE validation_labels RENAME TO labels")
            .unwrap();
        let (rows, issues, labels) = measured(&adapter, filter()).await;
        same(&rows, &direct(&conn, &filter()));
        assert_eq!((issues, labels), (6, 6));
        let (_, issues, labels) = measured(&adapter, filter()).await;
        assert_eq!((issues, labels), (0, 0));
    }
}

fn work_counter(capture: &Capture, name: &str) -> u64 {
    capture
        .events()
        .iter()
        .filter(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_issue_reads\""))
        .map(|e| {
            e.fields
                .0
                .get(name)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        })
        .sum()
}

#[tokio::test]
async fn issue_reads_over_budget_bypass_is_counted_on_each_request() {
    let (_dir, adapter, conn) = fixture(12).await;
    let cache_size: i64 = conn
        .query_row("PRAGMA cache_size", [], |r| r.get(0))
        .unwrap();
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let budget = if cache_size < 0 {
        -cache_size * 1024
    } else {
        cache_size * page_size
    };
    conn.execute(
        "UPDATE issues SET description=?1 WHERE id='bd-00000'",
        ["x".repeat(budget as usize + 1)],
    )
    .unwrap();
    let capture = Capture::default();
    for request in 1..=2 {
        let rows = adapter
            .list_issues(filter())
            .with_subscriber(capture.dispatch())
            .await
            .unwrap();
        same(&rows, &direct(&conn, &filter()));
        assert_eq!(
            work_counter(&capture, "over_budget_bypasses"),
            request,
            "every oversized result needs an explicit bypass reason"
        );
        assert_eq!(work_counter(&capture, "unsupported_order_bypasses"), 0);
        let work = adapter.issue_read_work();
        assert_eq!(work.full_builds, request);
        assert_eq!(
            (work.issue_rows, work.label_rows),
            (6 * request, 6 * request)
        );
        assert_eq!((work.subscriptions, work.retained_bytes), (0, 0));
    }
}

#[tokio::test]
async fn issue_reads_unsupported_order_bypass_is_counted_on_each_request() {
    let (_dir, adapter, conn) = fixture(12).await;
    // S3 does not decode created_at; a BLOB still has defined SQLite ordering.
    conn.execute("UPDATE issues SET created_at=x'80' WHERE id='bd-00000'", [])
        .unwrap();
    let capture = Capture::default();
    for request in 1..=2 {
        let rows = adapter
            .list_issues(filter())
            .with_subscriber(capture.dispatch())
            .await
            .unwrap();
        same(&rows, &direct(&conn, &filter()));
        assert_eq!(
            work_counter(&capture, "unsupported_order_bypasses"),
            request,
            "every unusual-order result needs an explicit bypass reason"
        );
        assert_eq!(work_counter(&capture, "over_budget_bypasses"), 0);
        let work = adapter.issue_read_work();
        assert_eq!(work.full_builds, request);
        assert_eq!(
            (work.issue_rows, work.label_rows),
            (6 * request, 6 * request)
        );
        assert_eq!((work.subscriptions, work.retained_bytes), (0, 0));
    }
}

#[path = "issue_reads_acceptance.rs"]
mod acceptance;
