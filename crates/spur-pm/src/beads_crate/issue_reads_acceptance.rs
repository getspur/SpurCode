//! Manual acceptance measurements. Durations are evidence, never CI thresholds.
use super::*;
use std::time::Instant;

fn emit(value: serde_json::Value) {
    eprintln!("S7_JSON {value}");
}

fn seed(conn: &Connection, size: usize, open: usize, open_labels: usize, total_labels: usize) {
    conn.execute_batch("BEGIN").unwrap();
    for i in 0..size {
        let (group, count, label_count, target) = if i < open {
            (i, open, open_labels, 491_643 * open / 396)
        } else {
            (
                i - open,
                size - open,
                total_labels - open_labels,
                3_447_421 * (size - open) / 1935,
            )
        };
        let nlabels = label_count / count + usize::from(group < label_count % count);
        let id = format!("bd-s7-{i:05}");
        let status = if i < open { "open" } else { "blocked" };
        let labels: Vec<_> = (0..nlabels)
            .map(|n| format!("synthetic-label-{n:012}"))
            .collect();
        // Match measured summary/raw sort/label UTF-8 bytes without private data.
        let fixed =
            id.len() * 2 + status.len() + 4 + 20 + labels.iter().map(String::len).sum::<usize>();
        let bytes = target / count + usize::from(group < target % count);
        let description = "x".repeat(bytes.saturating_sub(fixed));
        conn.execute("INSERT INTO issues(id,title,description,status,priority,issue_type,created_at,updated_at) VALUES(?1,?1,?2,?3,?4,'task','2026-10-09T00:00:00Z','2026-10-09T00:00:00Z')",rusqlite::params![id,description,status,i%5]).unwrap();
        for label in labels {
            conn.execute(
                "INSERT INTO labels(issue_id,label) VALUES(?1,?2)",
                rusqlite::params![id, label],
            )
            .unwrap();
        }
    }
    conn.execute_batch("COMMIT").unwrap();
}

async fn request(adapter: &BeadsCrateAdapter, filter: &IssueFilter, reuse: bool) -> Vec<u8> {
    let rows = if reuse {
        adapter.list_issues(filter.clone()).await.unwrap()
    } else {
        let filter = filter.clone();
        adapter
            .db
            .submit_summary_read(move |conn| {
                // Coherent snapshot; baseline does not inspect any feed metadata.
                let tx = conn.unchecked_transaction()?;
                let rows = crate::beads_crate::summary_query::list_summaries(&tx, &filter)?;
                tx.commit()?;
                Ok(rows)
            })
            .await
            .unwrap()
    };
    serde_json::to_vec(&rows).unwrap()
}

#[tokio::test]
#[ignore = "manual optimized whole-request and admission measurements"]
async fn s7_request_benchmark() {
    emit(
        serde_json::json!({"kind":"environment","profile":if cfg!(debug_assertions){"debug"}else{"release"},"sqlite":rusqlite::version(),"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"rustc":String::from_utf8_lossy(&std::process::Command::new("rustc").arg("-Vv").output().unwrap().stdout),"machine":String::from_utf8_lossy(&std::process::Command::new("uname").arg("-a").output().unwrap().stdout),"warmup":5,"repetitions":31,"boundary":"async reader queue through Vec<IssueSummary> and common JSON output serialization; direct baseline independent of feed"}),
    );
    for (size, open, open_labels, labels) in [(12, 6, 14, 38), (2331, 396, 943, 9674)] {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let conn = Connection::open(dir.path().join("beads.db")).unwrap();
        seed(&conn, size, open, open_labels, labels);
        for all in [false, true] {
            let f = IssueFilter {
                status: (!all).then(|| "open".into()),
                include_closed: all,
                ..Default::default()
            };
            let shape = if all {
                "all_non_template"
            } else {
                "open_unpaginated"
            };
            for _ in 0..5 {
                same_json(
                    &request(&adapter, &f, false).await,
                    &request(&adapter, &f, true).await,
                );
            }
            let before = adapter.issue_read_work();
            for round in 0..31 {
                let mut outputs = Vec::new();
                for reuse in if round % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    let output = request(&adapter, &f, reuse).await;
                    let ns = start.elapsed().as_nanos() as u64;
                    emit(
                        serde_json::json!({"kind":"request","size":size,"shape":shape,"round":round,"reuse":reuse,"ns":ns,"output_bytes":output.len()}),
                    );
                    outputs.push(output);
                }
                same_json(&outputs[0], &outputs[1]);
            }
            let after = adapter.issue_read_work();
            emit(
                serde_json::json!({"kind":"work","size":size,"shape":shape,"rows":if all{size}else{open},"labels":if all{labels}else{open_labels},"retained_bytes":after.retained_bytes,"budget_bytes":after.budget_bytes,"subscriptions":after.subscriptions,"full_builds":after.full_builds-before.full_builds,"issue_rows":after.issue_rows-before.issue_rows,"label_rows":after.label_rows-before.label_rows,"reuses":after.reuses-before.reuses,"over_budget_bypasses":after.over_budget_bypasses-before.over_budget_bypasses}),
            );
        }
    }
}

fn same_json(a: &[u8], b: &[u8]) {
    assert_eq!(a, b, "equivalent full-request output");
}

#[test]
#[ignore = "manual batch index/feed overhead and journal allocation measurements"]
fn s7_write_benchmark() {
    use beads_rust::storage::sqlite::SqliteStorage;
    for round in 0..9 {
        for mode in if round % 2 == 0 { [0, 1, 2] } else { [2, 1, 0] } {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("beads.db");
            drop(SqliteStorage::open(&path).unwrap());
            let mut conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
                .unwrap();
            conn.execute_batch(crate::beads_crate::sort_index::COMPOSITE_SORT_INDEX_SQL)
                .unwrap();
            if mode > 0 {
                crate::beads_crate::sort_index::ensure_composite_sort_index(&path).unwrap();
            }
            seed(&conn, 2331, 396, 943, 9674);
            if mode == 2 {
                crate::beads_crate::issue_changes::install(&mut conn).unwrap();
            }
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            // One warmup transaction on the same 200 IDs, then measured batches.
            for batch in 0..4 {
                let start = Instant::now();
                conn.execute_batch("BEGIN; UPDATE issues SET priority=(priority+1)%5, status=CASE status WHEN 'open' THEN 'blocked' ELSE 'open' END WHERE id IN (SELECT id FROM issues ORDER BY id LIMIT 200); COMMIT;").unwrap();
                let ns = start.elapsed().as_nanos() as u64;
                if batch > 0 {
                    emit(
                        serde_json::json!({"kind":"write","mode":mode,"round":round,"batch":batch,"batch_rows":200,"ns":ns,"sqlite":rusqlite::version()}),
                    );
                }
            }
            let start = Instant::now();
            conn.execute_batch("BEGIN; WITH RECURSIVE n(x) AS(SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<199) INSERT INTO issues(id,title,description,status,priority,issue_type,created_at,updated_at) SELECT printf('bd-new-%04d',x),'insert','synthetic','open',x%5,'task','2026-10-09T00:00:00Z','2026-10-09T00:00:00Z' FROM n; COMMIT;").unwrap();
            emit(
                serde_json::json!({"kind":"insert","mode":mode,"round":round,"batch_rows":200,"ns":start.elapsed().as_nanos() as u64}),
            );
            let page_size: i64 = conn
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .unwrap();
            let pages: i64 = conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .unwrap();
            let free: i64 = conn
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .unwrap();
            let journal_rows: i64 = if mode == 2 {
                conn.query_row("SELECT count(*) FROM spur_issue_changes", [], |r| r.get(0))
                    .unwrap()
            } else {
                0
            };
            let wal = std::fs::metadata(path.with_extension("db-wal"))
                .map(|m| m.len())
                .unwrap_or(0);
            if mode == 2 {
                conn.execute_batch("DROP TABLE spur_issue_changes").unwrap();
            }
            let free_after: i64 = conn
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .unwrap();
            let index_before = free_after;
            if mode > 0 {
                conn.execute_batch("DROP INDEX idx_issues_status_priority_created_at")
                    .unwrap();
            }
            let index_after: i64 = conn
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .unwrap();
            emit(
                serde_json::json!({"kind":"allocation","mode":mode,"round":round,"db_bytes":pages*page_size,"wal_bytes":wal,"journal_rows":journal_rows,"journal_table_and_indices_bytes":(free_after-free)*page_size,"status_index_bytes":(index_after-index_before)*page_size,"page_bytes":page_size}),
            );
        }
    }
}

fn site_count(capture: &Capture, site: &str) -> usize {
    let site = format!("\"{site}\"");
    capture
        .events()
        .iter()
        .filter(|e| e.fields.0.get("site") == Some(&site))
        .count()
}

#[tokio::test]
async fn s7_unchanged_requests_do_not_revalidate_ddl_or_reserialize_payload() {
    let (_dir, adapter, conn) = fixture(12).await;
    let cold = Capture::default();
    adapter
        .list_issues(filter())
        .with_subscriber(cold.dispatch())
        .await
        .unwrap();
    assert!(site_count(&cold, "beads_issue_schema_validate") > 0);
    assert_eq!(site_count(&cold, "beads_issue_snapshot_serialize"), 1);
    let warm = Capture::default();
    adapter
        .list_issues(filter())
        .with_subscriber(warm.dispatch())
        .await
        .unwrap();
    assert_eq!(
        site_count(&warm, "beads_issue_schema_validate"),
        0,
        "unchanged schema must not regenerate and compare DDL"
    );
    assert_eq!(
        site_count(&warm, "beads_issue_snapshot_serialize"),
        0,
        "unchanged retained payload must not be serialized again"
    );
    // A real schema change still invalidates exact DDL validation and reseeds.
    conn.execute_batch("CREATE TABLE s7_schema_changed(id INTEGER)")
        .unwrap();
    let changed = Capture::default();
    let rows = adapter
        .list_issues(filter())
        .with_subscriber(changed.dispatch())
        .await
        .unwrap();
    same(&rows, &direct(&conn, &filter()));
    assert!(site_count(&changed, "beads_issue_schema_validate") > 0);
    assert_eq!(site_count(&changed, "beads_issue_snapshot_serialize"), 1);
}

#[tokio::test]
async fn s7_structural_edits_advance_hygiene_without_summary_payload_loads() {
    let (_dir, adapter, conn) = fixture(12).await;
    let (before, _, _) = measured(&adapter, filter()).await;
    let cursor = adapter.hygiene_changes(None).await.unwrap().unwrap().cursor;
    conn.execute_batch("INSERT INTO dependencies(issue_id,depends_on_id,type,created_at,created_by) VALUES('bd-00000','bd-00001','PARENT-CHILD','2026-10-09T00:00:00Z','s7')").unwrap();
    let batch = adapter
        .hygiene_changes(Some(&cursor))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(batch.cursor, cursor);
    assert!(batch.affected_ids.contains(&"bd-00000".to_owned()));
    let (after, issues, labels) = measured(&adapter, filter()).await;
    same(&before, &after);
    assert_eq!((issues, labels), (0, 0));
    assert!(adapter
        .hygiene_changes(Some(&batch.cursor))
        .await
        .unwrap()
        .unwrap()
        .affected_ids
        .is_empty());
}

#[tokio::test]
async fn s7_unchanged_validated_revision_does_not_query_deltas() {
    let (_dir, adapter, conn) = fixture(12).await;
    measured(&adapter, filter()).await;
    let unchanged = Capture::default();
    adapter
        .list_issues(filter())
        .with_subscriber(unchanged.dispatch())
        .await
        .unwrap();
    assert_eq!(
        site_count(&unchanged, "beads_issue_delta_query"),
        0,
        "a validated equal revision has no delta rows to query"
    );
    conn.execute(
        "UPDATE issues SET description='delta' WHERE id='bd-00000'",
        [],
    )
    .unwrap();
    let changed = Capture::default();
    let rows = adapter
        .list_issues(filter())
        .with_subscriber(changed.dispatch())
        .await
        .unwrap();
    same(&rows, &direct(&conn, &filter()));
    assert_eq!(
        site_count(&changed, "beads_issue_delta_query"),
        1,
        "load delta once; validation still checks integrity and cursor"
    );
}
