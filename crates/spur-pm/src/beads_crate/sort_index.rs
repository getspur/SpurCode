//! Composite sort index for the beads graph snapshot hot path.
//!
//! `beads_rust`'s default issue ordering is `ORDER BY priority ASC,
//! created_at DESC`. Upstream schema (rev e07ccff) only ships single-column
//! `idx_issues_priority` / `idx_issues_created_at`, so every full-table
//! snapshot load ran `USE TEMP B-TREE FOR LAST TERM OF ORDER BY` — a sorter
//! over wide rows that spills to temp files once the payload exceeds
//! SQLite's default 2MB sorter memory (profiled 2026-10-07: 79–84% of
//! sampled non-wait activity, 14–23 MiB/s of sorter `pwrite`).
//!
//! Single-status issue lists also need a leading status key to satisfy
//! their filter and the default ordering without a temporary sort.
//!
//! Mitigation until the index lands upstream: best-effort
//! `CREATE INDEX IF NOT EXISTS` on connection open.

use std::path::Path;

/// Index satisfying `ORDER BY priority ASC, created_at DESC` (and, via
/// reverse scan, `priority DESC, created_at ASC`) without a sorter.
pub(crate) const COMPOSITE_SORT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_issues_priority_created_at \
     ON issues(priority ASC, created_at DESC);";

/// Index satisfying the default ordering within a single status.
const STATUS_COMPOSITE_SORT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_issues_status_priority_created_at \
     ON issues(status, priority ASC, created_at DESC);";

/// Best-effort creation of the composite sort indexes via a throwaway
/// connection. Requires the schema to already exist (call after
/// `SqliteStorage::open`, which creates it).
pub(crate) fn ensure_composite_sort_index(db_path: &Path) -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(COMPOSITE_SORT_INDEX_SQL)?;
    conn.execute_batch(STATUS_COMPOSITE_SORT_INDEX_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beads_crate::reader_pool::ReaderPool;
    use crate::beads_crate::{AdapterConfig, BeadsCrateAdapter};
    use beads_rust::storage::sqlite::SqliteStorage;

    fn seed_issues(conn: &rusqlite::Connection, count: i64) {
        conn.execute(
            "WITH RECURSIVE fixture(n) AS (
                SELECT 0 UNION ALL SELECT n + 1 FROM fixture WHERE n + 1 < ?1
             )
             INSERT INTO issues (id, title, description, status, priority, issue_type,
                                 created_at, updated_at, closed_at)
             SELECT printf('bd-sort-%05d', n), printf('Issue %d', n), printf('%0256d', n),
                    CASE n % 4 WHEN 0 THEN 'open' WHEN 1 THEN 'in_progress'
                               WHEN 2 THEN 'blocked' ELSE 'closed' END,
                    (n / 4) % 5, 'task',
                    printf('2026-10-%02dT00:00:00Z', (n / 20) % 3 + 1),
                    '2026-10-09T00:00:00Z',
                    CASE WHEN n % 4 = 3 THEN '2026-10-09T00:00:00Z' END FROM fixture",
            [count],
        )
        .unwrap();
    }

    fn assert_status_sort_plan(conn: &rusqlite::Connection, window: &str) {
        let mut stmt = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT * FROM issues WHERE status IN ('open') \
                 ORDER BY priority ASC, created_at DESC {window}"
            ))
            .unwrap();
        let plan = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join(" | ");
        assert!(
            !plan.to_lowercase().contains("temp b-tree"),
            "status-filtered sort ({window}) still uses a sorter: {plan}"
        );
        assert!(
            plan.contains("idx_issues_status_priority_created_at"),
            "status/order index not used ({window}): {plan}"
        );
    }

    fn check_status_sort_plan(window: &str) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        drop(SqliteStorage::open(&db_path).unwrap());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        seed_issues(&conn, 240);
        ensure_composite_sort_index(&db_path).unwrap();
        let fresh = rusqlite::Connection::open(&db_path).unwrap();
        assert_status_sort_plan(&fresh, window);
        // Execute a read before EXPLAIN so the seed connection observes the
        // schema change made by the helper's separate DDL connection.
        conn.query_row("SELECT count(*) FROM issues", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
        assert_status_sort_plan(&conn, window);
    }

    #[test]
    fn status_sort_plan_without_limit() {
        check_status_sort_plan("");
    }

    #[test]
    fn status_sort_plan_with_limit() {
        check_status_sort_plan("LIMIT 7");
    }

    #[tokio::test]
    async fn adapter_open_ensures_composite_sort_index() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        // Model an existing upstream database without Spur's sort index.
        drop(SqliteStorage::open(&db_path).unwrap());

        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert_status_sort_plan(&conn, "");
        assert_status_sort_plan(&conn, "LIMIT 7");
        for ordering in [
            "priority ASC, created_at DESC",
            "priority DESC, created_at ASC",
        ] {
            let mut stmt = conn
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN SELECT * FROM issues WHERE 1=1 ORDER BY {ordering}"
                ))
                .unwrap();
            let details: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let plan = details.join(" | ");
            assert!(
                !plan.to_lowercase().contains("temp b-tree"),
                "production adapter snapshot sort ({ordering}) still uses a sorter: {plan}"
            );
            assert!(
                plan.contains("idx_issues_priority_created_at"),
                "production adapter composite sort index not used ({ordering}): {plan}"
            );
        }
        adapter
            .read(|storage| {
                storage.list_issues(&Default::default())?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// RED → GREEN: checking out a reader connection must leave the DB with
    /// a composite index that removes the sorter from the default snapshot
    /// ordering (`USE TEMP B-TREE` absent from the query plan).
    #[tokio::test]
    async fn reader_checkout_ensures_composite_sort_index() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        // Prime schema via one open so ensure() never sees a missing table.
        drop(SqliteStorage::open(&db_path).unwrap());

        let pool = ReaderPool::new(dir.path().to_path_buf(), 1);
        {
            let _guard = pool.checkout().await.unwrap();
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert_status_sort_plan(&conn, "");
        assert_status_sort_plan(&conn, "LIMIT 7");
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT id FROM issues WHERE 1=1 \
                 ORDER BY priority ASC, created_at DESC",
            )
            .unwrap();
        let details: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let plan = details.join(" | ");
        assert!(
            !plan.to_lowercase().contains("temp b-tree"),
            "snapshot sort still uses a sorter: {plan}"
        );
        assert!(
            plan.contains("idx_issues_priority_created_at"),
            "composite sort index not used: {plan}"
        );
    }

    /// The helper itself is idempotent and safe to re-run on open.
    #[test]
    fn ensure_composite_sort_index_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        drop(SqliteStorage::open(&db_path).unwrap());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let index_names = || {
            conn.prepare("SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let original = index_names();
        ensure_composite_sort_index(&db_path).unwrap();
        let installed = index_names();
        ensure_composite_sort_index(&db_path).unwrap();
        assert_eq!(installed, index_names());
        assert!(original.iter().all(|name| installed.contains(name)));
        for name in [
            "idx_issues_priority_created_at",
            "idx_issues_status_priority_created_at",
        ] {
            assert!(
                installed.iter().any(|index| index == name),
                "missing {name}"
            );
        }
    }

    #[derive(Debug, PartialEq)]
    struct ListedRow {
        id: String,
        priority: i64,
        created_at: String,
        values: Vec<rusqlite::types::Value>,
    }

    fn list_rows(conn: &rusqlite::Connection, statuses: &str, window: &str) -> Vec<ListedRow> {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT * FROM issues WHERE status IN ({statuses}) \
                 ORDER BY priority ASC, created_at DESC {window}"
            ))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            Ok(ListedRow {
                id: row.get("id")?,
                priority: row.get("priority")?,
                created_at: row.get("created_at")?,
                values: (0..columns)
                    .map(|column| row.get(column))
                    .collect::<Result<_, _>>()?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn status_sort_preserves_rows_and_limit_windows_with_ties() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        drop(SqliteStorage::open(&db_path).unwrap());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        seed_issues(&conn, 240);
        // Preserve the pre-change schema, including Spur's existing sort index.
        conn.execute_batch(COMPOSITE_SORT_INDEX_SQL).unwrap();
        let statuses = [
            "'open'",
            "'open', 'in_progress'",
            "'closed'",
            "'open', 'in_progress', 'blocked', 'closed'",
        ];
        let windows: Vec<_> = [0, 1, 3, 15, 40, 239, 250]
            .into_iter()
            .flat_map(|offset| {
                [-1, 0, 1, 2, 7, 200]
                    .into_iter()
                    .map(move |limit| format!("LIMIT {limit} OFFSET {offset}"))
            })
            .collect();
        let before: Vec<_> = statuses
            .iter()
            .map(|statuses| {
                (
                    list_rows(&conn, statuses, ""),
                    windows
                        .iter()
                        .map(|window| list_rows(&conn, statuses, window))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        ensure_composite_sort_index(&db_path).unwrap();
        for (statuses, (mut all_before, pages_before)) in statuses.iter().zip(before) {
            let mut all_after = list_rows(&conn, statuses, "");
            // The SQL has no ID tie-breaker: compare sort keys positionally,
            // and full row contents as a set. A window may cut through a tie.
            let keys = |rows: &[ListedRow]| {
                rows.iter()
                    .map(|row| (row.priority, row.created_at.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(&all_before), keys(&all_after));
            assert!(all_before.windows(2).any(|pair| {
                pair[0].priority == pair[1].priority && pair[0].created_at == pair[1].created_at
            }));
            all_before.sort_by(|a, b| a.id.cmp(&b.id));
            all_after.sort_by(|a, b| a.id.cmp(&b.id));
            assert_eq!(all_before, all_after, "full rows for {statuses}");
            for (window, page_before) in windows.iter().zip(pages_before) {
                let page_after = list_rows(&conn, statuses, window);
                assert_eq!(
                    keys(&page_before),
                    keys(&page_after),
                    "{statuses}: {window}"
                );
                let ids: std::collections::HashSet<_> =
                    page_after.iter().map(|row| &row.id).collect();
                assert_eq!(ids.len(), page_after.len(), "duplicate rows: {window}");
                assert!(
                    page_after.iter().all(|row| all_before.contains(row)),
                    "{statuses}: {window}"
                );
            }
        }
    }

    /// Synthetic write cost only; timings are evidence, never pass/fail thresholds.
    #[test]
    #[ignore = "manual synthetic index footprint and write-cost measurement"]
    fn status_sort_index_write_cost() {
        use std::time::Instant;

        for round in 0..5 {
            // Alternate execution order to reduce warmup/order bias.
            for indexed in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let dir = tempfile::tempdir().unwrap();
                let db_path = dir.path().join("beads.db");
                drop(SqliteStorage::open(&db_path).unwrap());
                let conn = rusqlite::Connection::open(&db_path).unwrap();
                conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
                    .unwrap();
                conn.execute_batch(COMPOSITE_SORT_INDEX_SQL).unwrap();
                if indexed {
                    ensure_composite_sort_index(&db_path).unwrap();
                }
                let start = Instant::now();
                seed_issues(&conn, 10_000);
                let insert = start.elapsed();
                let start = Instant::now();
                conn.execute_batch(
                    "UPDATE issues SET priority = (priority + 1) % 5,
                        status = CASE status WHEN 'open' THEN 'in_progress' ELSE 'open' END,
                        created_at = '2026-10-09T00:00:00Z', closed_at = NULL;",
                )
                .unwrap();
                let update = start.elapsed();
                let pages: i64 = conn
                    .query_row("PRAGMA page_count", [], |row| row.get(0))
                    .unwrap();
                let page_size: i64 = conn
                    .query_row("PRAGMA page_size", [], |row| row.get(0))
                    .unwrap();
                let index_bytes = if indexed {
                    let free_before: i64 = conn
                        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
                        .unwrap();
                    conn.execute_batch("DROP INDEX idx_issues_status_priority_created_at")
                        .unwrap();
                    let free_after: i64 = conn
                        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
                        .unwrap();
                    (free_after - free_before) * page_size
                } else {
                    0
                };
                eprintln!(
                    "status-index fixture: sqlite={} round={round} indexed={indexed} rows=10000 \
                     insert_us={} update_us={} db_bytes={} index_bytes={index_bytes}",
                    rusqlite::version(),
                    insert.as_micros(),
                    update.as_micros(),
                    pages * page_size,
                );
            }
        }
    }
}
