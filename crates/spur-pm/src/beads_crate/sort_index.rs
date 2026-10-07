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
//! Mitigation until the index lands upstream: best-effort
//! `CREATE INDEX IF NOT EXISTS` on connection open.

use std::path::Path;

/// Index satisfying `ORDER BY priority ASC, created_at DESC` (and, via
/// reverse scan, `priority DESC, created_at ASC`) without a sorter.
pub(crate) const COMPOSITE_SORT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_issues_priority_created_at \
     ON issues(priority ASC, created_at DESC);";

/// Best-effort creation of the composite sort index via a throwaway
/// connection. Requires the schema to already exist (call after
/// `SqliteStorage::open`, which creates it).
pub(crate) fn ensure_composite_sort_index(db_path: &Path) -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(COMPOSITE_SORT_INDEX_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beads_crate::reader_pool::ReaderPool;
    use crate::beads_crate::{AdapterConfig, BeadsCrateAdapter};
    use beads_rust::storage::sqlite::SqliteStorage;

    #[tokio::test]
    async fn adapter_open_ensures_composite_sort_index() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("beads.db");
        // Model an existing upstream database without Spur's sort index.
        drop(SqliteStorage::open(&db_path).unwrap());

        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        adapter
            .read(|storage| {
                storage.list_issues(&Default::default())?;
                Ok(())
            })
            .await
            .unwrap();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
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
        ensure_composite_sort_index(&db_path).unwrap();
        ensure_composite_sort_index(&db_path).unwrap();
    }
}
