use super::*;
use petgraph::visit::EdgeRef;
use rusqlite::Connection;

fn fixture() -> (tempfile::TempDir, Connection, IncrementalGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beads.db");
    drop(beads_rust::storage::sqlite::SqliteStorage::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "PRAGMA foreign_keys=ON;
         INSERT INTO issues(id,title) VALUES ('a','A'),('b','B'),('c','C');
         INSERT INTO labels(issue_id,label) VALUES ('a','one'),('b','one'),('c','two');
         INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES ('b','a','blocks'),('c','b','parent-child');",
    ).unwrap();
    let store = IncrementalGraphStore::new(path, 5_000);
    (dir, db, store)
}

fn graph(
    store: &mut IncrementalGraphStore,
    label: Option<&str>,
) -> (Vec<String>, Vec<String>, String) {
    store
        .query(label, |snap| {
            let mut nodes = snap
                .graph
                .node_weights()
                .map(|n| {
                    format!(
                        "{}|{}|{}|{}|{}|{:?}|{:?}|{}|{}|{:?}",
                        n.id,
                        n.title,
                        n.status,
                        n.priority,
                        n.issue_type,
                        n.assignee,
                        n.labels,
                        n.created_at,
                        n.updated_at,
                        n.due_at
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = snap
                .graph
                .edge_references()
                .map(|e| {
                    format!(
                        "{}|{}|{:?}",
                        snap.graph[e.source()].id,
                        snap.graph[e.target()].id,
                        e.weight().kind
                    )
                })
                .collect::<Vec<_>>();
            nodes.sort();
            edges.sort();
            (nodes, edges, snap.data_hash.clone())
        })
        .unwrap()
}

#[test]
fn unchanged_and_unrelated_writes_do_not_reload_graph_data() {
    let (_dir, db, mut store) = fixture();
    let before = graph(&mut store, None);
    let work = store.metrics;
    for sql in [
        "INSERT INTO comments(issue_id,author,text) VALUES ('a','test','comment')",
        "INSERT INTO config(key,value) VALUES ('unrelated','1')",
        "UPDATE issues SET title=title WHERE id='a'",
        "UPDATE dependencies SET type=type WHERE issue_id='b'",
        "UPDATE labels SET label=label WHERE issue_id='a'",
    ] {
        db.execute_batch(sql).unwrap();
        assert_eq!(graph(&mut store, None), before);
    }
    assert_eq!(store.metrics.full_builds, 1);
    assert_eq!(store.metrics.nodes_loaded, work.nodes_loaded);
    assert_eq!(store.metrics.edges_loaded, work.edges_loaded);
    assert_eq!(store.metrics.delta_refreshes, work.delta_refreshes);
    assert_eq!(store.metrics.reuses, 5);
}

#[test]
fn external_single_node_edit_loads_only_that_node() {
    let (_dir, db, mut store) = fixture();
    let before = graph(&mut store, None);
    let work = store.metrics;
    db.execute("UPDATE issues SET title='Changed' WHERE id='b'", [])
        .unwrap();
    let after = graph(&mut store, None);
    assert!(after.0.iter().any(|s| s.starts_with("b|Changed|")));
    assert_ne!(before.2, after.2);
    assert_eq!(store.metrics.full_builds, work.full_builds);
    assert_eq!(store.metrics.nodes_loaded - work.nodes_loaded, 1);
    assert_eq!(store.metrics.edges_loaded, work.edges_loaded);
    assert_eq!(store.metrics.delta_refreshes, 1);
    db.execute("UPDATE issues SET title='B' WHERE id='b'", [])
        .unwrap();
    assert_eq!(
        graph(&mut store, None),
        before,
        "fingerprint is content-based, not history-based"
    );
}

#[test]
fn edge_mutations_do_not_reload_nodes_and_cover_type_and_endpoints() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    let work = store.metrics;
    for sql in [
        "UPDATE dependencies SET type='related-to' WHERE issue_id='b'",
        "UPDATE dependencies SET depends_on_id='c' WHERE issue_id='b'",
        "DELETE FROM dependencies WHERE issue_id='b'",
        "INSERT INTO dependencies(issue_id,depends_on_id,type) VALUES ('a','c','waits-for')",
    ] {
        db.execute_batch(sql).unwrap();
        let incremental = graph(&mut store, None);
        let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
        assert_eq!(incremental, graph(&mut cold, None));
    }
    assert_eq!(store.metrics.full_builds, 1);
    assert_eq!(store.metrics.nodes_loaded, work.nodes_loaded);
}

#[test]
fn label_membership_and_outside_changes_are_scoped() {
    let (_dir, db, mut store) = fixture();
    let before = graph(&mut store, Some("one"));
    assert_eq!(before.0.len(), 2);
    let work = store.metrics;
    db.execute("UPDATE issues SET title='Outside edit' WHERE id='c'", [])
        .unwrap();
    assert_eq!(graph(&mut store, Some("one")), before);
    assert_eq!(store.metrics.nodes_loaded, work.nodes_loaded);
    db.execute("INSERT INTO labels(issue_id,label) VALUES ('c','one')", [])
        .unwrap();
    let expanded = graph(&mut store, Some("one"));
    assert_eq!(expanded.0.len(), 3);
    assert_eq!(expanded.1.len(), 2);
    db.execute("DELETE FROM labels WHERE issue_id='b' AND label='one'", [])
        .unwrap();
    let contracted = graph(&mut store, Some("one"));
    assert_eq!(contracted.0.len(), 2);
    assert!(contracted.1.is_empty());
    let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
    assert_eq!(contracted, graph(&mut cold, Some("one")));
    assert_eq!(store.metrics.full_builds, 1);
}

#[test]
fn deleting_a_dense_graph_node_repairs_indices_and_reintroduction_restores_edges() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    db.execute("DELETE FROM issues WHERE id='b'", []).unwrap();
    let removed = graph(&mut store, None);
    assert_eq!(removed.0.len(), 2);
    assert!(removed.1.is_empty());
    db.execute("UPDATE issues SET title='C updated' WHERE id='c'", [])
        .unwrap();
    db.execute("INSERT INTO issues(id,title) VALUES ('b','B restored')", [])
        .unwrap();
    let restored = graph(&mut store, None);
    assert_eq!(restored.0.len(), 3);
    assert!(restored.0.iter().any(|n| n.starts_with("c|C updated|")));
    assert_eq!(restored.1, vec!["b|c|ParentChild"]);
    let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
    assert_eq!(restored, graph(&mut cold, None));
    assert_eq!(store.metrics.full_builds, 1);
}

#[test]
fn tombstones_templates_and_label_updates_remove_and_restore_membership() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    for sql in [
        "UPDATE issues SET status='tombstone' WHERE id='b'",
        "UPDATE issues SET status='open',is_template=1 WHERE id='b'",
        "UPDATE issues SET is_template=0 WHERE id='b'",
        "UPDATE labels SET issue_id='c',label='moved' WHERE issue_id='a'",
    ] {
        db.execute_batch(sql).unwrap();
        let incremental = graph(&mut store, None);
        let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
        assert_eq!(incremental, graph(&mut cold, None));
    }
    assert_eq!(store.metrics.full_builds, 1);
}

#[test]
fn rollback_does_not_advance_graph_revision() {
    let (_dir, db, mut store) = fixture();
    let before = graph(&mut store, None);
    let work = store.metrics;
    db.execute_batch("BEGIN; UPDATE issues SET title='rolled back' WHERE id='a'; DELETE FROM dependencies; ROLLBACK;").unwrap();
    assert_eq!(graph(&mut store, None), before);
    assert_eq!(store.metrics.nodes_loaded, work.nodes_loaded);
    assert_eq!(store.metrics.edges_loaded, work.edges_loaded);
    assert_eq!(store.metrics.delta_refreshes, 0);
}

#[test]
fn failed_delta_does_not_publish_partial_data_or_advance_cursor() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    db.execute_batch("BEGIN; UPDATE issues SET title='A changed' WHERE id='a'; UPDATE issues SET created_at='invalid' WHERE id='b'; COMMIT;").unwrap();
    let before = store.metrics;
    assert!(store.query(None, |_| ()).is_err());
    assert_eq!(store.metrics, before);
    db.execute(
        "UPDATE issues SET created_at='2026-10-09 00:00:00' WHERE id='b'",
        [],
    )
    .unwrap();
    let after = graph(&mut store, None);
    assert!(after.0.iter().any(|n| n.starts_with("a|A changed|")));
    let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
    assert_eq!(after, graph(&mut cold, None));
    assert_eq!(store.metrics.full_builds, 1);
}

#[test]
fn independent_consumers_do_not_steal_changes() {
    let (_dir, db, mut first) = fixture();
    let mut second = IncrementalGraphStore::new(first.path.clone(), 5_000);
    graph(&mut first, None);
    graph(&mut second, None);
    db.execute("UPDATE issues SET priority=0 WHERE id='a'", [])
        .unwrap();
    let one = graph(&mut first, None);
    assert_eq!(one, graph(&mut second, None));
    assert_eq!(first.metrics.full_builds, 1);
    assert_eq!(second.metrics.full_builds, 1);
}

#[test]
fn schema_recovery_reinstalls_tracking_and_rebuilds_once() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    db.execute_batch("DROP TRIGGER spur_graph_issues_update; UPDATE issues SET title='missed while tracking absent' WHERE id='a';").unwrap();
    let recovered = graph(&mut store, None);
    assert!(recovered
        .0
        .iter()
        .any(|n| n.contains("missed while tracking absent")));
    assert_eq!(store.metrics.full_builds, 2);
    db.execute("UPDATE issues SET title='tracked again' WHERE id='a'", [])
        .unwrap();
    let result = graph(&mut store, None);
    assert!(result.0.iter().any(|n| n.contains("tracked again")));
    assert_eq!(store.metrics.full_builds, 2);
}

#[test]
fn replace_conflict_records_the_deleted_issue_even_without_recursive_triggers() {
    let (_dir, db, mut store) = fixture();
    db.execute("UPDATE issues SET external_ref='shared' WHERE id='a'", [])
        .unwrap();
    graph(&mut store, None);
    db.execute_batch("PRAGMA recursive_triggers=OFF; INSERT OR REPLACE INTO issues(id,title,external_ref) VALUES ('replacement','Replacement','shared');").unwrap();
    let updated = graph(&mut store, None);
    assert!(!updated.0.iter().any(|n| n.starts_with("a|")));
    assert!(updated.0.iter().any(|n| n.starts_with("replacement|")));
    let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
    assert_eq!(updated, graph(&mut cold, None));
    assert_eq!(store.metrics.full_builds, 1);
}

#[test]
fn a_large_graph_still_loads_one_node_for_one_field_change() {
    let (_dir, mut db, mut store) = fixture();
    let tx = db.transaction().unwrap();
    for i in 0..2_000 {
        tx.execute(
            "INSERT INTO issues(id,title) VALUES (?1,'Unrelated')",
            [format!("large-{i}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(store.query(None, |s| s.node_count()).unwrap(), 2_003);
    let work = store.metrics;
    for _ in 0..10 {
        assert_eq!(store.query(None, |s| s.node_count()).unwrap(), 2_003);
    }
    let warm = store.metrics;
    assert_eq!(warm.full_builds, work.full_builds);
    assert_eq!(warm.nodes_loaded, work.nodes_loaded);
    assert_eq!(warm.edges_loaded, work.edges_loaded);
    assert_eq!(warm.reuses - work.reuses, 10);
    db.execute("UPDATE issues SET title='Only this one' WHERE id='b'", [])
        .unwrap();
    store
        .query(None, |s| {
            assert_eq!(s.graph[s.by_id["b"]].title, "Only this one");
        })
        .unwrap();
    assert_eq!(store.metrics.full_builds, 1);
    assert_eq!(store.metrics.nodes_loaded - work.nodes_loaded, 1);
    assert_eq!(store.metrics.edges_loaded, work.edges_loaded);
    let observation = |scenario: &str, value: GraphWorkMetrics| {
        serde_json::json!({
            "scenario":scenario, "full_builds":value.full_builds,
            "delta_refreshes":value.delta_refreshes, "reuses":value.reuses,
            "changed_ids":value.changed_ids, "nodes_loaded":value.nodes_loaded,
            "edges_loaded":value.edges_loaded
        })
    };
    println!(
        "GRAPH_WORK_SAMPLE {}",
        serde_json::json!({
            "fixture_nodes":2_003, "fixture_edges":2,
            "observations":[observation("cold", work),
                observation("ten_unchanged_reads", warm),
                observation("one_node_edit", store.metrics)]
        })
    );
}

fn oracle_projection(snap: &GraphSnapshot) -> (Vec<String>, Vec<String>) {
    let mut nodes = snap
        .graph
        .node_weights()
        .map(|n| {
            let mut labels = n.labels.clone();
            labels.sort();
            serde_json::json!({"id":n.id,"title":n.title,"status":n.status,
            "priority":n.priority,"type":n.issue_type,"assignee":n.assignee,
            "created_at":n.created_at,"updated_at":n.updated_at,"due_at":n.due_at,
            "labels":labels})
            .to_string()
        })
        .collect::<Vec<_>>();
    let mut edges = snap
        .graph
        .edge_references()
        .map(|e| {
            format!(
                "{}|{}|{:?}",
                snap.graph[e.source()].id,
                snap.graph[e.target()].id,
                e.weight().kind
            )
        })
        .collect::<Vec<_>>();
    nodes.sort();
    edges.sort();
    (nodes, edges)
}

#[test]
fn incremental_projection_matches_the_existing_full_loader() {
    let (_dir, db, mut store) = fixture();
    let legacy = beads_rust::storage::sqlite::SqliteStorage::open(&store.path).unwrap();
    for sql in [
        "SELECT 1",
        "UPDATE issues SET assignee='',issue_type='custom-type' WHERE id='a'",
        "UPDATE issues SET status='OPEN',issue_type='FEATURE' WHERE id='a'",
        "UPDATE issues SET status='inprogress',issue_type='CUSTOM-TYPE' WHERE id='a'",
        "UPDATE dependencies SET type='BLOCKS' WHERE issue_id='b'",
        "UPDATE issues SET status='TOMBSTONE' WHERE id='b'",
        "UPDATE issues SET status='open' WHERE id='b'",
        "UPDATE issues SET priority=0,title='Changed',due_at='2026-10-20 00:00:00' WHERE id='a'",
        "INSERT INTO labels(issue_id,label) VALUES ('c','one')",
        "UPDATE dependencies SET type='related' WHERE issue_id='b'",
        "UPDATE dependencies SET depends_on_id='a',type='waits-for' WHERE issue_id='c'",
        "UPDATE issues SET status='tombstone' WHERE id='b'",
        "UPDATE issues SET status='open' WHERE id='b'",
        "DELETE FROM issues WHERE id='a'",
    ] {
        db.execute_batch(sql).unwrap();
        for label in [None, Some("one")] {
            let expected =
                crate::graph_engine::snapshot::load_graph_snapshot(&legacy, label).unwrap();
            let actual = store.query(label, oracle_projection).unwrap();
            assert_eq!(
                actual,
                oracle_projection(&expected),
                "after {sql}, scope {label:?}"
            );
        }
    }
    assert_eq!(store.metrics.full_builds, 2);
}

#[test]
fn empty_label_initialization_uses_indexed_queries() {
    let (_dir, db, mut store) = fixture();
    for sql in [
        initial_nodes_sql(Some("unknown")),
        initial_edges_sql(Some("unknown")),
    ] {
        let plan = db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(["unknown"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !plan.iter().any(|step| step.starts_with("SCAN issues")
                || step.starts_with("SCAN dependencies")
                || step.starts_with("SCAN labels")),
            "unbounded scan in {plan:?}"
        );
    }
    for _ in 0..3 {
        assert!(graph(&mut store, Some("unknown")).0.is_empty());
    }
    assert_eq!(store.metrics.nodes_loaded, 0);
    assert_eq!(store.metrics.edges_loaded, 0);
}

#[test]
fn scoped_refresh_ignores_corrupt_outside_dependency_payloads() {
    let (_dir, db, mut store) = fixture();
    let before = graph(&mut store, Some("one"));
    // c owns this incoming edge to b but is outside the requested label.
    db.execute("UPDATE dependencies SET type=x'ff' WHERE issue_id='c'", [])
        .unwrap();
    assert_eq!(graph(&mut store, Some("one")), before);
    let mut cold = IncrementalGraphStore::new(store.path.clone(), 5_000);
    assert_eq!(graph(&mut cold, Some("one")), before);
    // Once c joins the scope its invalid payload must be reported, with the
    // previously published view and cursor intact.
    db.execute("INSERT INTO labels(issue_id,label) VALUES ('c','one')", [])
        .unwrap();
    let work = store.metrics;
    assert!(store.query(Some("one"), |_| ()).is_err());
    assert_eq!(store.metrics, work);
}

#[test]
fn replacement_database_reopens_and_initializes_the_new_graph() {
    let (_dir, db, mut store) = fixture();
    graph(&mut store, None);
    drop(db);
    let other = tempfile::tempdir().unwrap();
    let replacement = other.path().join("fresh.db");
    drop(beads_rust::storage::sqlite::SqliteStorage::open(&replacement).unwrap());
    let new = Connection::open(&replacement).unwrap();
    new.execute("INSERT INTO issues(id,title) VALUES ('fresh','Fresh')", [])
        .unwrap();
    new.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(new);
    // Close the old WAL connection before replacement; replacing open SQLite
    // files is outside SQLite's guarantees. Reopen a checkpointed copy to keep
    // the previous connection's schema/revision and cached graph intact. Without
    // identity detection the next query would reuse the obsolete cached graph.
    let mut old = store.connected.take().unwrap();
    drop(std::mem::replace(
        &mut old.conn,
        Connection::open_in_memory().unwrap(),
    ));
    let previous = other.path().join("previous.db");
    std::fs::copy(&store.path, &previous).unwrap();
    old.conn = Connection::open(&previous).unwrap();
    std::fs::rename(&replacement, &store.path).unwrap();
    assert!(old.identity != FileIdentity::read(&store.path).unwrap());
    store.connected = Some(old);
    let fresh = graph(&mut store, None);
    assert_eq!(fresh.0.len(), 1);
    assert!(fresh.0[0].starts_with("fresh|Fresh|"));
    assert_eq!(store.metrics.full_builds, 2);
    assert_eq!(store.metrics.recoveries, 1);
}
