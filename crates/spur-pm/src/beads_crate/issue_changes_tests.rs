use super::*;
use std::collections::BTreeSet;

fn fixture() -> (tempfile::TempDir, Connection, IssueChangeFeed) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beads.db");
    drop(beads_rust::storage::sqlite::SqliteStorage::open(&path).unwrap());
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("INSERT INTO issues(id,title) VALUES ('a','A'),('b','B'),('c','C');")
        .unwrap();
    install(&mut db).unwrap();
    (dir, db, IssueChangeFeed::new(&path, 5_000))
}

fn batch(feed: &mut IssueChangeFeed, since: Option<&IssueCursor>) -> IssueChanges {
    feed.read(since, |_, b| Ok(b.clone())).unwrap()
}
fn ids(b: &IssueChanges, domain: fn(&IssueChange) -> bool) -> BTreeSet<&str> {
    b.changes
        .iter()
        .filter(|c| domain(c))
        .map(|c| c.issue_id.as_str())
        .collect()
}
fn expect_ids(b: &IssueChanges, expected: &[&str], domain: fn(&IssueChange) -> bool) {
    assert!(!b.reseed, "valid tracking must allow a delta");
    assert_eq!(ids(b, domain), expected.iter().copied().collect());
}

#[test]
fn external_description_edit_with_fixed_timestamp_and_hash_is_visible() {
    let (_dir, db, mut feed) = fixture();
    let before = batch(&mut feed, None).cursor;
    let metadata: (String, Option<String>) = db
        .query_row(
            "SELECT updated_at,content_hash FROM issues WHERE id='a'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute(
        "UPDATE issues SET description='changed description' WHERE id='a'",
        [],
    )
    .unwrap();
    let after = batch(&mut feed, Some(&before));
    assert!(
        after.cursor.revision > before.revision,
        "description-only external writes must advance the issue watermark"
    );
    expect_ids(&after, &["a"], |c| c.summary);
    assert_eq!(
        db.query_row(
            "SELECT updated_at,content_hash FROM issues WHERE id='a'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        )
        .unwrap(),
        metadata
    );
}

#[test]
fn label_add_remove_rename_and_reparent_report_both_ids() {
    let (_dir, db, mut feed) = fixture();
    for (sql, expected) in [
        ("INSERT INTO labels VALUES ('a','one')", vec!["a"]),
        (
            "UPDATE labels SET label='two' WHERE issue_id='a'",
            vec!["a"],
        ),
        (
            "UPDATE labels SET issue_id='b' WHERE issue_id='a'",
            vec!["a", "b"],
        ),
        ("DELETE FROM labels WHERE issue_id='b'", vec!["b"]),
    ] {
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(sql).unwrap();
        expect_ids(&batch(&mut feed, Some(&before)), &expected, |c| c.summary);
    }
}

#[test]
fn insert_tombstone_delete_and_id_change_retain_deletion_evidence() {
    let (_dir, db, mut feed) = fixture();
    let old = batch(&mut feed, None).cursor;
    db.execute_batch("INSERT INTO issues(id,title) VALUES ('new','New'); UPDATE issues SET status='tombstone' WHERE id='b'; UPDATE issues SET id='renamed' WHERE id='a'; DELETE FROM issues WHERE id='c';").unwrap();
    let changes = batch(&mut feed, Some(&old));
    expect_ids(&changes, &["a", "b", "c", "new", "renamed"], |c| c.summary);
    assert_eq!(ids(&changes, |c| c.deleted), BTreeSet::from(["a", "c"]));
    assert!(
        changes
            .changes
            .iter()
            .find(|c| c.issue_id == "b")
            .unwrap()
            .issue
    );
}

#[test]
fn replace_insert_and_update_include_external_ref_victims_without_recursive_triggers() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch(
        "PRAGMA recursive_triggers=OFF; UPDATE issues SET external_ref='shared' WHERE id='a';",
    )
    .unwrap();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch(
        "INSERT OR REPLACE INTO issues(id,title,external_ref) VALUES ('replacement','R','shared');",
    )
    .unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a", "replacement"], |c| c.summary);
    assert!(changes
        .changes
        .iter()
        .any(|c| c.issue_id == "a" && c.deleted));
    let before = changes.cursor;
    db.execute_batch("UPDATE OR REPLACE issues SET external_ref='shared' WHERE id='b'; UPDATE OR REPLACE issues SET id='c' WHERE id='b';").unwrap();
    expect_ids(
        &batch(&mut feed, Some(&before)),
        &["b", "c", "replacement"],
        |c| c.summary,
    );
}

#[test]
fn comment_and_audit_mutations_are_separate_from_summaries() {
    let (_dir, db, mut feed) = fixture();
    for table in ["comments", "events"] {
        let insertion = if table == "comments" {
            "INSERT INTO comments(id,issue_id,author,text) VALUES (10,'a','test','hello')"
        } else {
            "INSERT INTO events(id,issue_id,event_type) VALUES (10,'a','updated')"
        };
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(insertion).unwrap();
        let changes = batch(&mut feed, Some(&before));
        expect_ids(&changes, &["a"], |c| c.comments);
        expect_ids(&changes, &[], |c| c.summary);
        let before = changes.cursor;
        db.execute_batch(&format!("UPDATE {table} SET issue_id='b' WHERE id=10;"))
            .unwrap();
        let changes = batch(&mut feed, Some(&before));
        expect_ids(&changes, &["a", "b"], |c| c.comments);
        let before = changes.cursor;
        db.execute_batch(&format!("DELETE FROM {table} WHERE id=10;"))
            .unwrap();
        expect_ids(&batch(&mut feed, Some(&before)), &["b"], |c| c.comments);
    }
}

#[test]
fn comment_replace_records_old_owner_even_when_delete_triggers_are_disabled() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch("PRAGMA recursive_triggers=OFF; INSERT INTO comments(id,issue_id,author,text) VALUES(1,'a','t','old'),(2,'c','t','old');").unwrap();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("INSERT OR REPLACE INTO comments(id,issue_id,author,text) VALUES(1,'b','t','new'); UPDATE OR REPLACE comments SET id=2 WHERE id=1;").unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a", "b", "c"], |c| c.comments);
    expect_ids(&changes, &[], |c| c.summary);
}

#[test]
fn every_summary_filter_order_field_is_tracked_and_other_issue_fields_reach_hygiene() {
    let (_dir, db, mut feed) = fixture();
    for assignment in [
        "title='new'",
        "description='new'",
        "status='in_progress'",
        "priority=1",
        "issue_type='bug'",
        "assignee='x'",
        "created_at='2000-01-01'",
        "is_template=1",
    ] {
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(&format!("UPDATE issues SET {assignment} WHERE id='a';"))
            .unwrap();
        expect_ids(&batch(&mut feed, Some(&before)), &["a"], |c| c.summary);
    }
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("UPDATE issues SET notes='hygiene',due_at='2000-01-03' WHERE id='a'")
        .unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a"], |c| c.issue);
    expect_ids(&changes, &[], |c| c.summary);
}

#[test]
fn dependency_edits_are_hygiene_only_and_rollbacks_do_not_advance_the_feed() {
    let (_dir, db, mut feed) = fixture();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("INSERT INTO dependencies(issue_id,depends_on_id) VALUES ('a','b'); UPDATE dependencies SET type='parent-child'; UPDATE issues SET description=description WHERE id='a'; BEGIN; UPDATE issues SET title='rolled back'; INSERT INTO comments(issue_id,author,text) VALUES('a','t','rolled back'); ROLLBACK;").unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a"], |c| c.issue);
    expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
}

#[test]
fn coalescing_retains_older_domain_changes_for_lagging_independent_readers() {
    let (_dir, db, mut feed) = fixture();
    let lagging = batch(&mut feed, None).cursor;
    db.execute_batch("UPDATE issues SET description='v1' WHERE id='a'")
        .unwrap();
    let recent = batch(&mut feed, Some(&lagging)).cursor;
    db.execute_batch("INSERT INTO comments(issue_id,author,text) VALUES ('a','t','comment'); DELETE FROM issues WHERE id='b';").unwrap();
    let old = batch(&mut feed, Some(&lagging));
    expect_ids(&old, &["a", "b"], |c| c.summary);
    expect_ids(&batch(&mut feed, Some(&recent)), &["b"], |c| c.summary);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM spur_issue_changes WHERE issue_id='a'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(old.changes.iter().any(|c| c.issue_id == "b" && c.deleted));
}

#[test]
fn lost_or_corrupt_tracking_forces_reseed_and_recovers() {
    for sabotage in [
        "DROP TABLE spur_issue_changes",
        "DROP TABLE spur_issue_clock",
        "DELETE FROM spur_issue_clock",
        "UPDATE spur_issue_clock SET revision=999999",
        "DELETE FROM spur_issue_changes",
        "UPDATE spur_issue_changes SET summary_revision=0",
        "UPDATE spur_issue_changes SET timestamp_revision=0",
        "DROP TRIGGER spur_issue_issues_update",
        "DROP TRIGGER spur_issue_issues_update; CREATE TRIGGER spur_issue_issues_update AFTER UPDATE ON issues BEGIN SELECT 1; END",
        "CREATE TABLE schema_drift(value TEXT)",
    ] {
        let (_dir, db, mut feed) = fixture();
        db.execute_batch("UPDATE issues SET description='tracked' WHERE id='a'").unwrap();
        let before = batch(&mut feed, None).cursor;
        let installed: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='spur_issue_changes')", [], |r| r.get(0)).unwrap();
        assert!(installed, "recovery requires the durable journal to be installed");
        db.execute_batch(sabotage).unwrap();
        if sabotage.contains("DROP TRIGGER") {
            db.execute_batch("UPDATE issues SET description='untracked gap' WHERE id='a'").unwrap();
        }
        let recovered = batch(&mut feed, Some(&before));
        assert!(recovered.reseed, "must reseed after {sabotage}");
        assert_ne!(recovered.cursor, before);
        // Explicit same-writer recovery; the dedicated journal test asserts the
        // rejected post-repair write and unchanged row before this schema read.
        db.query_row("SELECT name FROM sqlite_schema WHERE type='table' AND name='spur_issue_changes'", [], |_| Ok(())).unwrap();
        db.execute_batch("UPDATE issues SET description='after recovery' WHERE id='c'").unwrap();
        expect_ids(&batch(&mut feed, Some(&recovered.cursor)), &["c"], |c| c.summary);
    }
}

#[test]
fn watermark_delta_and_loaded_rows_share_one_snapshot() {
    let (_dir, db, mut feed) = fixture();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("UPDATE issues SET description='first' WHERE id='a'")
        .unwrap();
    let snapshot = feed
        .read(Some(&before), |tx, b| {
            assert!(b.cursor.revision > before.revision);
            db.execute_batch("UPDATE issues SET description='racing' WHERE id='a'")?;
            let text: String =
                tx.query_row("SELECT description FROM issues WHERE id='a'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(text, "first");
            Ok(b.clone())
        })
        .unwrap();
    let next = batch(&mut feed, Some(&snapshot.cursor));
    assert!(next.cursor.revision > snapshot.cursor.revision);
    expect_ids(&next, &["a"], |c| c.summary);
}

#[test]
fn reconnect_preserves_cursor_and_database_replacement_requires_reseed() {
    let (dir, db, mut feed) = fixture();
    let path = dir.path().join("beads.db");
    let before = batch(&mut feed, None).cursor;
    drop(feed);
    db.execute_batch("UPDATE issues SET description='while disconnected' WHERE id='a'")
        .unwrap();
    let mut feed = IssueChangeFeed::new(&path, 5_000);
    let after = batch(&mut feed, Some(&before));
    expect_ids(&after, &["a"], |c| c.summary);
    // VACUUM INTO copies the durable generation too: physical identity must win.
    let replacement = dir.path().join("replacement.db");
    db.execute("VACUUM INTO ?", [replacement.to_str().unwrap()])
        .unwrap();
    drop(db);
    std::fs::rename(&replacement, &path).unwrap();
    assert!(batch(&mut feed, Some(&after.cursor)).reseed);
}

#[tokio::test(flavor = "multi_thread")]
async fn adapter_installs_feed_before_external_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let _adapter = crate::beads_crate::BeadsCrateAdapter::open(
        dir.path(),
        crate::beads_crate::AdapterConfig::default(),
    )
    .await
    .unwrap();
    let db = Connection::open(dir.path().join("beads.db")).unwrap();
    let installed: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='spur_issue_clock')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        installed,
        "adapter open must install the durable feed before returning"
    );
}

#[test]
fn hidden_rowid_replace_victims_are_not_lost() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch("PRAGMA recursive_triggers=OFF;").unwrap();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch(
        "UPDATE OR REPLACE issues SET rowid=(SELECT rowid FROM issues WHERE id='a') WHERE id='b';",
    )
    .unwrap();
    let changes = batch(&mut feed, Some(&before));
    assert!(
        changes
            .changes
            .iter()
            .any(|c| c.issue_id == "a" && c.deleted && c.summary),
        "hidden rowid replacement must report the deleted issue victim"
    );
    let before = changes.cursor;
    db.execute_batch("INSERT OR REPLACE INTO issues(rowid,id,title) SELECT rowid,'new','New' FROM issues WHERE id='c';").unwrap();
    expect_ids(&batch(&mut feed, Some(&before)), &["c", "new"], |c| {
        c.summary
    });
}

#[test]
fn label_rowid_replace_reports_the_displaced_owner() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch(
        "PRAGMA recursive_triggers=OFF; INSERT INTO labels VALUES('a','one'),('b','two');",
    )
    .unwrap();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("UPDATE OR REPLACE labels SET rowid=(SELECT rowid FROM labels WHERE issue_id='a') WHERE issue_id='b';").unwrap();
    assert!(
        ids(&batch(&mut feed, Some(&before)), |c| c.summary).contains("a"),
        "hidden label rowid victim's owner must be reported"
    );
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("INSERT OR REPLACE INTO labels(rowid,issue_id,label) SELECT rowid,'c','new' FROM labels WHERE issue_id='b';").unwrap();
    expect_ids(&batch(&mut feed, Some(&before)), &["b", "c"], |c| c.summary);
}

#[tokio::test(flavor = "multi_thread")]
async fn adapter_comment_timestamp_is_unrelated_to_plain_summaries() {
    use crate::BeadsAdvanced;

    let dir = tempfile::tempdir().unwrap();
    let adapter = crate::beads_crate::BeadsCrateAdapter::open(
        dir.path(),
        crate::beads_crate::AdapterConfig::default(),
    )
    .await
    .unwrap();
    let path = dir.path().join("beads.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "INSERT INTO issues(id,title,updated_at) VALUES('a','A','2000-01-01T00:00:00Z')",
    )
    .unwrap();
    let mut feed = IssueChangeFeed::new(&path, 5_000);
    let before = batch(&mut feed, None).cursor;

    let comment_id = adapter
        .add_comment("a", "ordinary adapter comment")
        .await
        .unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a"], |c| c.comments);
    expect_ids(&changes, &["a"], |c| c.issue);
    assert!(
        ids(&changes, |c| c.summary).is_empty(),
        "adapter.add_comment only changes comments and updated_at; since=None must not reload summary payloads"
    );
    expect_ids(&changes, &["a"], |c| c.timestamp);
    let (timestamp, text): (String, String) = db.query_row(
        "SELECT issues.updated_at,comments.text FROM issues JOIN comments ON comments.issue_id=issues.id WHERE comments.id=?1",
        [&comment_id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_ne!(timestamp, "2000-01-01T00:00:00Z");
    assert_eq!(text, "ordinary adapter comment");
}

#[test]
fn timestamp_summary_and_comment_revisions_survive_compaction_in_both_orders() {
    for timestamp_first in [false, true] {
        let (_dir, db, mut feed) = fixture();
        let lagging = batch(&mut feed, None).cursor;
        let timestamp_sql = "UPDATE issues SET updated_at='2001-01-01T00:00:00Z' WHERE id='a'";
        let summary_sql = "UPDATE issues SET description='new payload' WHERE id='a'";
        db.execute_batch(if timestamp_first {
            timestamp_sql
        } else {
            summary_sql
        })
        .unwrap();
        let middle = batch(&mut feed, Some(&lagging)).cursor;
        db.execute_batch(if timestamp_first {
            summary_sql
        } else {
            timestamp_sql
        })
        .unwrap();
        db.execute_batch(
            "INSERT INTO comments(issue_id,author,text) VALUES('a','t','last domain')",
        )
        .unwrap();
        let recent = batch(&mut feed, Some(&middle));
        expect_ids(&recent, if timestamp_first { &["a"] } else { &[] }, |c| {
            c.summary
        });
        expect_ids(&recent, if timestamp_first { &[] } else { &["a"] }, |c| {
            c.timestamp
        });
        let old = batch(&mut feed, Some(&lagging));
        expect_ids(&old, &["a"], |c| c.timestamp);
        expect_ids(&old, &["a"], |c| c.summary);
        expect_ids(&old, &["a"], |c| c.comments);
        let (timestamp_revision, summary_revision, comment_revision): (i64,i64,i64) = db.query_row(
            "SELECT timestamp_revision,summary_revision,comment_revision FROM spur_issue_changes WHERE issue_id='a'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        assert!(timestamp_revision > lagging.revision);
        assert!(summary_revision > lagging.revision);
        assert_eq!(timestamp_revision > middle.revision, !timestamp_first);
        assert_eq!(summary_revision > middle.revision, timestamp_first);
        assert_eq!(comment_revision, old.cursor.revision);
    }
}

#[test]
fn repaired_journal_rejects_stale_writer_then_schema_read_allows_tracked_retry() {
    eprintln!("recovery SQLite {}", rusqlite::version());
    for mode in ["WAL", "DELETE"] {
        let (_dir, db, mut feed) = fixture();
        db.pragma_update(None, "journal_mode", mode).unwrap();
        db.execute_batch("UPDATE issues SET description='before repair' WHERE id='a'")
            .unwrap();
        let before = batch(&mut feed, None).cursor;
        db.execute_batch("DROP TABLE spur_issue_changes").unwrap();
        let recovered = batch(&mut feed, Some(&before));
        assert!(recovered.reseed);
        // Deliberately AFTER repair: reproduce the original execute_batch path
        // with a fresh statement, not a previously prepared/cached UPDATE.
        let error = db
            .execute_batch("UPDATE issues SET description='rejected' WHERE id='a'")
            .expect_err("fresh post-repair statement must expose the stale writer schema");
        eprintln!("{mode} post-repair rejection: {error}");
        assert!(
            error
                .to_string()
                .contains("no such table: main.spur_issue_changes"),
            "{mode}: {error}"
        );
        feed.read(Some(&recovered.cursor), |tx, changes| {
            expect_ids(changes, &[], |_| true);
            let value: String =
                tx.query_row("SELECT description FROM issues WHERE id='a'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(
                value, "before repair",
                "rejected write must leave the row unchanged"
            );
            Ok(())
        })
        .unwrap();
        db.query_row("SELECT 1", [], |_| Ok(())).unwrap();
        db.query_row("PRAGMA schema_version", [], |_| Ok(()))
            .unwrap();
        assert!(db
            .execute_batch("UPDATE issues SET description='still rejected' WHERE id='a'")
            .is_err());
        db.query_row(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name='spur_issue_changes'",
            [],
            |_| Ok(()),
        )
        .unwrap();
        db.execute_batch("UPDATE issues SET description='tracked retry' WHERE id='a'")
            .unwrap();
        expect_ids(&batch(&mut feed, Some(&recovered.cursor)), &["a"], |c| {
            c.summary
        });
        assert_eq!(
            db.query_row("SELECT description FROM issues WHERE id='a'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "tracked retry"
        );
    }
}

#[test]
fn exact_trigger_restoration_cannot_hide_an_untracked_edit() {
    let (_dir, db, mut feed) = fixture();
    let before = batch(&mut feed, None).cursor;
    let trigger: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='spur_issue_issues_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER spur_issue_issues_update; UPDATE issues SET description='missed' WHERE id='a';").unwrap();
    db.execute_batch(&trigger).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='spur_issue_issues_update'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        trigger
    );
    let recovered = feed
        .read(Some(&before), |tx, b| {
            assert!(
                b.reseed,
                "exact DDL restoration must not hide the changed schema cookie"
            );
            assert_eq!(
                tx.query_row("SELECT description FROM issues WHERE id='a'", [], |r| {
                    r.get::<_, String>(0)
                })?,
                "missed"
            );
            Ok(b.clone())
        })
        .unwrap();
    db.execute_batch("UPDATE issues SET description='tracked again' WHERE id='b'")
        .unwrap();
    expect_ids(&batch(&mut feed, Some(&recovered.cursor)), &["b"], |c| {
        c.summary
    });
}

#[test]
fn serialized_cursor_after_feed_restart_detects_clone_replacement() {
    let (dir, db, mut feed) = fixture();
    let path = dir.path().join("beads.db");
    let cursor = batch(&mut feed, None).cursor;
    let saved = serde_json::to_string(&cursor).unwrap();
    drop(feed);
    let cursor: IssueCursor = serde_json::from_str(&saved).unwrap();
    let mut feed = IssueChangeFeed::new(&path, 5_000);
    assert_eq!(batch(&mut feed, Some(&cursor)).cursor, cursor);
    db.execute_batch("UPDATE issues SET description='offline edit' WHERE id='a'")
        .unwrap();
    expect_ids(&batch(&mut feed, Some(&cursor)), &["a"], |c| c.summary);
    let replacement = dir.path().join("clone.db");
    drop(feed);
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(db);
    // Copy the checkpointed file byte-for-byte, including UUID/schema cookie.
    std::fs::copy(&path, &replacement).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    let mut feed = IssueChangeFeed::new(&path, 5_000);
    let replaced = batch(&mut feed, Some(&cursor));
    assert!(replaced.reseed);
    assert_eq!(
        replaced.cursor.generation, cursor.generation,
        "clone retains UUID; physical identity must detect replacement"
    );
    assert_ne!(replaced.cursor.identity, cursor.identity);
}

#[test]
fn ignored_and_failed_conflicts_preserve_rows_and_complete_feed() {
    for conflict in ["IGNORE", "ABORT", "FAIL", "ROLLBACK"] {
        for insert in [true, false] {
            let (_dir, db, mut feed) = fixture();
            db.execute_batch("PRAGMA recursive_triggers=OFF; UPDATE issues SET external_ref='shared' WHERE id='a'").unwrap();
            let before = batch(&mut feed, None).cursor;
            let sql = if insert {
                format!("INSERT OR {conflict} INTO issues(id,title,external_ref) VALUES('new','N','shared')")
            } else {
                format!("UPDATE OR {conflict} issues SET external_ref='shared' WHERE id='b'")
            };
            let result = db.execute(&sql, []);
            if conflict == "IGNORE" {
                assert_eq!(result.unwrap(), 0);
            } else {
                assert!(result.is_err());
            }
            assert_eq!(
                db.query_row("SELECT count(*) FROM issues", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                3
            );
            assert_eq!(
                db.query_row(
                    "SELECT id FROM issues WHERE external_ref='shared'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "a"
            );
            let changes = batch(&mut feed, Some(&before));
            assert!(!changes.reseed);
            assert!(ids(&changes, |c| c.deleted).is_empty());
            if ["ABORT", "ROLLBACK"].contains(&conflict) {
                assert_eq!(
                    changes.cursor, before,
                    "aborting conflicts must roll back victim capture"
                );
            } else {
                // IGNORE/FAIL can preserve BEFORE writes, but must only cause
                // conservative invalidation and must not poison later tracking.
                expect_ids(&changes, &["a"], |c| c.summary);
            }
            db.execute_batch("UPDATE issues SET description='later' WHERE id='c'")
                .unwrap();
            expect_ids(&batch(&mut feed, Some(&changes.cursor)), &["c"], |c| {
                c.summary
            });
        }
    }
}

// Execute the shared production queries with safe statement work counters.
// Metadata/trigger SQL generation has fixed schema-dependent cost; S7 measures
// its latency. These checks bound issue/journal row work, not wall-clock time.
fn query_work(db: &Connection, sql: &str, params: &[i64]) -> (usize, i32, i32, i32) {
    use rusqlite::StatementStatus;
    let mut stmt = db.prepare(sql).unwrap();
    let mut rows = stmt.query(rusqlite::params_from_iter(params)).unwrap();
    let mut count = 0;
    while rows.next().unwrap().is_some() {
        count += 1;
    }
    drop(rows);
    (
        count,
        stmt.get_status(StatementStatus::VmStep),
        stmt.get_status(StatementStatus::FullscanStep),
        stmt.get_status(StatementStatus::Sort),
    )
}

#[test]
fn unchanged_validation_and_delta_work_do_not_scale_with_issue_rows() {
    let mut baseline = None;
    let mut changed_baseline = None;
    for size in [3, 300, 3_000] {
        let (_dir, mut db, mut feed) = fixture();
        let tx = db.transaction().unwrap();
        for n in 3..size {
            tx.execute(
                "INSERT INTO issues(id,title) VALUES(?1,'bulk')",
                [format!("bulk-{n}")],
            )
            .unwrap();
        }
        tx.execute_batch("INSERT INTO labels SELECT id,'bulk' FROM issues; UPDATE issues SET notes='seed journal';").unwrap();
        tx.commit().unwrap();
        let cursor = batch(&mut feed, None).cursor;
        let unchanged = batch(&mut feed, Some(&cursor));
        expect_ids(&unchanged, &[], |_| true);
        assert_eq!(unchanged.cursor, cursor);
        let clock = query_work(&db, CLOCK_QUERY, &[cursor.schema, i64::from(VERSION)]);
        let delta = query_work(&db, CHANGES_QUERY, &[cursor.revision]);
        assert_eq!((clock.0, clock.2, clock.3), (1, 0, 0));
        assert_eq!((delta.0, delta.2), (0, 0));
        let work = (clock, delta);
        eprintln!("watermark/delta work size={size}: {work:?} (rows, vm, fullscan, sorts)");
        if let Some(baseline) = baseline {
            assert_eq!(
                work, baseline,
                "unchanged watermark/delta work must stay fixed"
            );
        } else {
            baseline = Some(work);
        }
        db.execute_batch("UPDATE issues SET description='single edit' WHERE id='a'")
            .unwrap();
        expect_ids(&batch(&mut feed, Some(&cursor)), &["a"], |c| c.summary);
        let changed_work = query_work(&db, CHANGES_QUERY, &[cursor.revision]);
        assert_eq!((changed_work.0, changed_work.2), (1, 0));
        if let Some(baseline) = changed_baseline {
            assert_eq!(changed_work, baseline);
        } else {
            changed_baseline = Some(changed_work);
        }
        for (query, params) in [
            (CLOCK_QUERY, vec![cursor.schema, i64::from(VERSION)]),
            (CHANGES_QUERY, vec![cursor.revision]),
        ] {
            let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
            let plan = explain
                .query_map(rusqlite::params_from_iter(params), |r| {
                    r.get::<_, String>(3)
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                plan.iter()
                    .any(|p| p.contains("spur_issue_changes_revision")),
                "{plan:?}"
            );
            assert!(
                !plan.iter().any(|p| p.starts_with("SCAN issues")
                    || p.starts_with("SCAN labels")
                    || (query == CHANGES_QUERY && p.starts_with("SCAN spur_issue_changes"))),
                "{plan:?}"
            );
        }
    }
}

#[test]
fn timestamp_only_edits_signal_since_membership_entry_and_exit() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch("UPDATE issues SET updated_at='2000-01-01T00:00:00Z' WHERE id='a'")
        .unwrap();
    let mut cursor = batch(&mut feed, None).cursor;
    let membership_sql = "SELECT updated_at >= '2001-01-01T00:00:00Z' FROM issues WHERE id='a'";
    assert!(!db
        .query_row(membership_sql, [], |r| r.get::<_, bool>(0))
        .unwrap());
    for (updated_at, expected_member) in [
        ("2002-01-01T00:00:00Z", true),
        ("2000-01-01T00:00:00Z", false),
    ] {
        db.execute("UPDATE issues SET updated_at=?1 WHERE id='a'", [updated_at])
            .unwrap();
        let changes = feed
            .read(Some(&cursor), |tx, b| {
                expect_ids(b, &[], |c| c.summary);
                expect_ids(b, &["a"], |c| c.timestamp);
                expect_ids(b, &["a"], |c| c.issue);
                expect_ids(b, &[], |c| c.comments);
                assert_eq!(
                    tx.query_row(membership_sql, [], |r| r.get::<_, bool>(0))?,
                    expected_member
                );
                Ok(b.clone())
            })
            .unwrap();
        assert!(
            changes.cursor.revision > cursor.revision,
            "watermark advances even when updated_at moves backwards"
        );
        cursor = changes.cursor;
    }
}

#[test]
fn structural_edges_track_old_new_children_and_ignore_blockers() {
    let (_dir, db, mut feed) = fixture();
    for (sql, children) in [
        (
            "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','b','blocks')",
            vec![],
        ),
        ("UPDATE dependencies SET type='parent-child'", vec!["a"]),
        ("UPDATE dependencies SET depends_on_id='c'", vec!["a"]),
        ("UPDATE dependencies SET issue_id='b'", vec!["a", "b"]),
        ("UPDATE dependencies SET type='blocks'", vec!["b"]),
        ("DELETE FROM dependencies", vec![]),
        (
            "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','parent-child')",
            vec!["a"],
        ),
        ("BEGIN; DELETE FROM dependencies; ROLLBACK", vec![]),
        ("DELETE FROM dependencies", vec!["a"]),
    ] {
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(sql).unwrap();
        let changes = batch(&mut feed, Some(&before));
        expect_ids(&changes, &children, |c| c.issue);
        expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
    }
}

#[test]
fn structural_replace_captures_rowid_and_key_victims_without_recursive_triggers() {
    for insert in [true, false] {
        let (_dir, db, mut feed) = fixture();
        db.execute_batch("PRAGMA recursive_triggers=OFF; INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','parent-child'),('b','c','blocks')").unwrap();
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(if insert {
            "INSERT OR REPLACE INTO dependencies(rowid,issue_id,depends_on_id,type) SELECT rowid,'c','b','blocks' FROM dependencies WHERE issue_id='a'"
        } else {
            "UPDATE OR REPLACE dependencies SET rowid=(SELECT rowid FROM dependencies WHERE issue_id='a') WHERE issue_id='b'"
        }).unwrap();
        let changes = batch(&mut feed, Some(&before));
        expect_ids(&changes, &["a"], |c| c.issue);
        expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
    }
}

#[test]
fn structural_combined_retarget_type_change_marks_both_children() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch(
        "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','parent-child')",
    )
    .unwrap();
    let before = batch(&mut feed, None).cursor;
    db.execute_batch("UPDATE dependencies SET issue_id='b',type='blocks'")
        .unwrap();
    let changes = batch(&mut feed, Some(&before));
    expect_ids(&changes, &["a", "b"], |c| c.issue);
    expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
}

#[test]
fn hygiene_review_mixed_case_structural_changes_track_old_and_new_children() {
    let (_dir, db, mut feed) = fixture();
    for (sql, children) in [
        (
            "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','PARENT-CHILD')",
            vec!["a"],
        ),
        ("UPDATE dependencies SET depends_on_id='b'", vec!["a"]),
        ("UPDATE dependencies SET issue_id='c'", vec!["a", "c"]),
        (
            "UPDATE dependencies SET issue_id='a',type='blocks'",
            vec!["a", "c"],
        ),
        (
            "UPDATE dependencies SET issue_id='c',type='PaReNt-ChIlD'",
            vec!["a", "c"],
        ),
        ("BEGIN; DELETE FROM dependencies; ROLLBACK", vec![]),
        ("DELETE FROM dependencies", vec!["c"]),
    ] {
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(sql).unwrap();
        let changes = batch(&mut feed, Some(&before));
        assert_eq!(
            ids(&changes, |c| c.issue),
            children.into_iter().collect(),
            "{sql}"
        );
        assert!(!changes.reseed);
        expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
    }
}

#[test]
fn hygiene_review_mixed_case_replace_captures_rowid_and_composite_key_victims() {
    for sql in [
        "INSERT OR REPLACE INTO dependencies(rowid,issue_id,depends_on_id,type) SELECT rowid,'c','b','blocks' FROM dependencies WHERE issue_id='a'",
        "UPDATE OR REPLACE dependencies SET rowid=(SELECT rowid FROM dependencies WHERE issue_id='a') WHERE issue_id='b'",
        // These two conflict on (issue_id, depends_on_id), without a rowid conflict.
        "INSERT OR REPLACE INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','blocks')",
        "UPDATE OR REPLACE dependencies SET issue_id='a' WHERE issue_id='b'",
    ] {
        let (_dir, db, mut feed) = fixture();
        db.execute_batch("PRAGMA recursive_triggers=OFF; INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','c','PaReNt-ChIlD'),('b','c','blocks')").unwrap();
        let before = batch(&mut feed, None).cursor;
        db.execute_batch(sql).unwrap();
        let changes = batch(&mut feed, Some(&before));
        assert_eq!(ids(&changes, |c| c.issue), BTreeSet::from(["a"]), "{sql}");
        assert!(!changes.reseed);
        expect_ids(&changes, &[], |c| c.summary || c.timestamp || c.comments);
    }
}
