use super::*;
use rusqlite::Connection;

fn fixture() -> (tempfile::TempDir, Connection, IssueChangeFeed) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beads.db");
    drop(beads_rust::storage::sqlite::SqliteStorage::open(&path).unwrap());
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("INSERT INTO issues(id,title,status,closed_at) VALUES('a','A','open',NULL),('b','B','open',NULL),('parent','Parent','closed','2000-01-01T00:00:00Z'); INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES('a','parent','parent-child'),('b','parent','blocks')").unwrap();
    super::super::issue_changes::install(&mut db).unwrap();
    (dir, db, IssueChangeFeed::new(&path, 5000))
}
fn candidates(batch: &HygieneBatch) -> BTreeSet<&str> {
    batch
        .candidates
        .iter()
        .map(|c| c.issue.id.as_str())
        .collect()
}

#[test]
fn hygiene_snapshot_expands_closed_parent_and_filters_deleted_or_ineligible_children() {
    let (_dir, db, mut feed) = fixture();
    let seed = read(&mut feed, None).unwrap();
    assert!(seed.reseed);
    assert_eq!(candidates(&seed), BTreeSet::from(["a", "b"]));
    let unchanged = read(&mut feed, Some(&seed.cursor)).unwrap();
    assert!(!unchanged.reseed);
    assert!(unchanged.candidates.is_empty());
    db.execute_batch("INSERT INTO comments(issue_id,author,text) VALUES('parent','t','new audit')")
        .unwrap();
    let delta = read(&mut feed, Some(&seed.cursor)).unwrap();
    assert_eq!(candidates(&delta), BTreeSet::from(["a"]));
    assert_eq!(delta.candidates[0].parent_ids, vec!["parent"]);
    assert_eq!(delta.affected_ids, vec!["a", "parent"]);
    db.execute_batch(
        "UPDATE issues SET status='closed',closed_at='2000-01-01T00:00:00Z' WHERE id='a'; DELETE FROM issues WHERE id='b'",
    )
    .unwrap();
    let exit = read(&mut feed, Some(&delta.cursor)).unwrap();
    assert!(exit.candidates.is_empty());
    assert_eq!(exit.affected_ids, vec!["a", "b"]);
    db.execute_batch("UPDATE issues SET status='open',closed_at=NULL WHERE id='a'")
        .unwrap();
    let entry = read(&mut feed, Some(&exit.cursor)).unwrap();
    assert_eq!(candidates(&entry), BTreeSet::from(["a"]));
    db.execute_batch("UPDATE dependencies SET type='blocks' WHERE issue_id='a'")
        .unwrap();
    let structural = read(&mut feed, Some(&entry.cursor)).unwrap();
    assert_eq!(candidates(&structural), BTreeSet::from(["a"]));
    assert!(structural.candidates[0].parent_ids.is_empty());
}

#[test]
fn hygiene_history_gap_reseeds_and_parent_lookup_is_indexed() {
    let (_dir, db, mut feed) = fixture();
    let old = read(&mut feed, None).unwrap();
    db.execute_batch("DROP TABLE spur_issue_changes").unwrap();
    let repaired = read(&mut feed, Some(&old.cursor)).unwrap();
    assert!(repaired.reseed);
    assert_ne!(repaired.cursor, old.cursor);
    assert_eq!(candidates(&repaired), BTreeSet::from(["a", "b"]));
    let mut stmt = db
        .prepare(&format!("EXPLAIN QUERY PLAN {CHILDREN}"))
        .unwrap();
    let plan = stmt
        .query_map(["parent"], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            // With case-normalized types SQLite may choose the narrower
            // depends_on index. Both indexes bound work to this parent's edges.
            .any(|p| p.starts_with("SEARCH dependencies ") && p.contains("(depends_on_id=?)")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|p| p.contains("SCAN dependencies")),
        "{plan:?}"
    );
}

#[test]
fn hygiene_structural_conflicts_rollback_or_conservatively_invalidate() {
    for conflict in ["IGNORE", "FAIL", "ABORT", "ROLLBACK", "REPLACE"] {
        for insert in [false, true] {
            let (_dir, db, mut feed) = fixture();
            let before = read(&mut feed, None).unwrap();
            let sql = if insert {
                format!("INSERT OR {conflict} INTO dependencies(issue_id,depends_on_id,type) VALUES('a','parent','blocks')")
            } else {
                format!("UPDATE OR {conflict} dependencies SET issue_id='a' WHERE issue_id='b'")
            };
            let result = db.execute(&sql, []);
            assert_eq!(result.is_ok(), matches!(conflict, "IGNORE" | "REPLACE"));
            let after = read(&mut feed, Some(&before.cursor)).unwrap();
            assert!(!after.reseed);
            if matches!(conflict, "ABORT" | "ROLLBACK") {
                assert_eq!(after.cursor, before.cursor);
            } else {
                assert_eq!(candidates(&after), BTreeSet::from(["a"]));
            }
        }
    }
}

#[test]
fn hygiene_review_parent_endpoints_match_graph_eligibility() {
    let (_dir, db, mut feed) = fixture();
    // The current upstream schema requires status, but graph readers also
    // support nullable legacy/external rows. Keep every column and its values
    // while allowing those rows in this private fixture; the feed reseeds on DDL.
    db.execute_batch("PRAGMA foreign_keys=OFF; CREATE TABLE nullable_issues AS SELECT * FROM issues; DROP TABLE issues; ALTER TABLE nullable_issues RENAME TO issues").unwrap();
    let mut cursor = read(&mut feed, None).unwrap().cursor;
    for (status, template, eligible) in [
        (Some("closed"), Some(0), true),
        (Some("deferred"), Some(0), true),
        (Some("custom"), Some(0), true),
        (None, None, true),
        (Some(" TOMBSTONE "), Some(0), true),
        (Some("ToMbStOnE"), Some(0), false),
        (Some("closed"), Some(1), false),
    ] {
        db.execute(
            "UPDATE issues SET status=?1,is_template=?2,closed_at=CASE WHEN ?1='closed' THEN '2000-01-01T00:00:00Z' ELSE NULL END WHERE id='parent'",
            rusqlite::params![status, template],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO comments(issue_id,author,text) VALUES('parent','t','audit changed')",
        )
        .unwrap();
        let delta = read(&mut feed, Some(&cursor)).unwrap();
        let child = delta.candidates.iter().find(|c| c.issue.id == "a").unwrap();
        assert_eq!(
            child.parent_ids,
            if eligible { vec!["parent"] } else { vec![] },
            "status={status:?}, template={template:?}"
        );
        cursor = delta.cursor;
    }
    // External writers can retain dependencies and comments after endpoint loss.
    db.execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM issues WHERE id='parent'")
        .unwrap();
    let delta = read(&mut feed, Some(&cursor)).unwrap();
    assert_eq!(candidates(&delta), BTreeSet::from(["a"]));
    assert!(delta.candidates[0].parent_ids.is_empty());
}

#[test]
fn hygiene_review_mixed_case_parent_comment_expands_to_child() {
    let (_dir, db, mut feed) = fixture();
    db.execute_batch("UPDATE dependencies SET type='PaReNt-ChIlD' WHERE issue_id='a'")
        .unwrap();
    let seed = read(&mut feed, None).unwrap();
    db.execute_batch("INSERT INTO comments(issue_id,author,text) VALUES('parent','t','new audit')")
        .unwrap();
    let delta = read(&mut feed, Some(&seed.cursor)).unwrap();
    assert_eq!(candidates(&delta), BTreeSet::from(["a"]));
    assert_eq!(delta.candidates[0].parent_ids, vec!["parent"]);
    assert_eq!(delta.affected_ids, vec!["a", "parent"]);
}
