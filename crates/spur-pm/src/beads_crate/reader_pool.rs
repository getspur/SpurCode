//! Small fixed pool of `beads_rust::SqliteStorage` reader connections.
//!
//! WAL mode permits concurrent readers via *multiple* connections. One
//! connection cannot be called concurrently. The pool gives us bounded
//! concurrency without unbounded handle growth.
//!
//! Capacity is enforced by an async semaphore: at most `capacity` permits
//! are outstanding at any time, so at most `capacity` connections exist.
//! Connections are opened lazily on first miss and reused thereafter.
//!
//! ## Threading note
//!
//! `ReaderGuard` and its storage handle are kept on the thread that checked
//! them out. Consequences:
//!   - A `ReaderGuard` (and the `SqliteStorage` it wraps) cannot move across
//!     thread boundaries.
//!   - `ReaderPool` itself is `!Send`, so it cannot be cloned into a
//!     `tokio::spawn`'d task on a multi-threaded runtime.
//!   - All pool usage must stay on a single thread (e.g., `current_thread`
//!     runtime, `LocalSet` / `spawn_local`, or a dedicated worker thread that
//!     owns the pool).
//!
//! The semaphore still serializes capacity correctly under cooperative
//! single-threaded concurrency: while one task is awaiting the permit, others
//! can hold guards and progress.

use std::path::PathBuf;
use std::sync::Mutex;

use beads_rust::storage::sqlite::SqliteStorage;
use tokio::sync::{Semaphore, SemaphorePermit};

pub struct ReaderPool {
    beads_dir: PathBuf,
    free: Mutex<Vec<SqliteStorage>>,
    capacity: Semaphore,
}

impl ReaderPool {
    pub fn new(beads_dir: PathBuf, capacity: usize) -> Self {
        assert!(capacity > 0, "ReaderPool capacity must be > 0");
        Self {
            beads_dir,
            free: Mutex::new(Vec::with_capacity(capacity)),
            capacity: Semaphore::new(capacity),
        }
    }

    /// Check out a reader connection. Awaits a capacity permit, then either
    /// reuses a free connection or opens a new one. The returned guard holds
    /// the permit; dropping it returns the connection and releases capacity.
    pub async fn checkout(&self) -> anyhow::Result<ReaderGuard<'_>> {
        let permit = self
            .capacity
            .acquire()
            .await
            .expect("ReaderPool semaphore never closed");
        let capacity_trace =
            crate::lock_trace::LockTraceGuard::lock("reader_pool.capacity", "ReaderPool::checkout");
        let cached = {
            let _free_trace =
                crate::lock_trace::LockTraceGuard::lock("reader_pool.free", "ReaderPool::checkout");
            let mut free = self.free.lock().unwrap();
            free.pop()
        };
        let conn = match cached {
            Some(c) => c,
            None => {
                let db_path = self.beads_dir.join("beads.db");
                let conn = SqliteStorage::open(&db_path)?;
                // Schema exists by now (`SqliteStorage::open` creates it), so
                // the composite sort index can be ensured best-effort. Only
                // new connections pay this check (once per pool slot, not per
                // checkout); `IF NOT EXISTS` makes repeats a schema no-op.
                if let Err(err) =
                    crate::beads_crate::sort_index::ensure_composite_sort_index(&db_path)
                {
                    tracing::warn!(
                        ?err,
                        ?db_path,
                        "composite sort index ensure failed (best-effort)"
                    );
                }
                conn
            }
        };
        let conn_trace =
            crate::lock_trace::LockTraceGuard::conn("reader_pool.sqlite", "ReaderPool::checkout");
        Ok(ReaderGuard {
            pool: self,
            conn: Some(conn),
            _permit: permit,
            _capacity_trace: capacity_trace,
            _conn_trace: conn_trace,
        })
    }
}

pub struct ReaderGuard<'a> {
    pool: &'a ReaderPool,
    conn: Option<SqliteStorage>,
    _permit: SemaphorePermit<'a>,
    _capacity_trace: crate::lock_trace::LockTraceGuard,
    _conn_trace: crate::lock_trace::LockTraceGuard,
}

impl<'a> ReaderGuard<'a> {
    pub fn storage(&self) -> &SqliteStorage {
        self.conn
            .as_ref()
            .expect("guard always has conn before drop")
    }
}

impl<'a> Drop for ReaderGuard<'a> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let _free_trace =
                crate::lock_trace::LockTraceGuard::lock("reader_pool.free", "ReaderGuard::drop");
            let mut free = self.pool.free.lock().unwrap();
            free.push(conn);
        }
        // _permit drop releases the semaphore slot.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    fn init_beads(dir: &std::path::Path) {
        let _ = SqliteStorage::open(&dir.join("beads.db")).expect("open initializes schema");
    }

    #[tokio::test]
    async fn checkout_returns_a_reader() {
        let dir = TempDir::new().unwrap();
        init_beads(dir.path());
        let pool = ReaderPool::new(dir.path().to_path_buf(), 2);
        let r = pool.checkout().await.expect("checkout");
        let _ = r.storage();
    }

    #[tokio::test]
    async fn drop_returns_to_pool() {
        let dir = TempDir::new().unwrap();
        init_beads(dir.path());
        let pool = ReaderPool::new(dir.path().to_path_buf(), 1);
        {
            let _ = pool.checkout().await.unwrap();
        }
        assert_eq!(pool.free.lock().unwrap().len(), 1);
    }

    /// With capacity=1 and one outstanding guard, a second checkout must NOT
    /// resolve until the first guard is dropped. SqliteStorage is `!Send`, so
    /// we cannot use `tokio::spawn` — we drive the second future cooperatively
    /// via `pin!` + `timeout`.
    #[tokio::test]
    async fn checkout_blocks_when_at_capacity() {
        let dir = TempDir::new().unwrap();
        init_beads(dir.path());
        let pool = ReaderPool::new(dir.path().to_path_buf(), 1);

        let g1 = pool.checkout().await.expect("first checkout");

        let second = pool.checkout();
        tokio::pin!(second);

        // While g1 is held, polling the second future to completion must time out.
        let blocked = tokio::time::timeout(Duration::from_millis(50), &mut second).await;
        assert!(
            blocked.is_err(),
            "second checkout completed while permit was held"
        );

        drop(g1);

        // After g1 is dropped, the second future must resolve promptly.
        let g2 = tokio::time::timeout(Duration::from_millis(500), &mut second)
            .await
            .expect("second checkout did not unblock after first was dropped")
            .expect("checkout returned an error");
        let _ = g2.storage();
    }

    /// Hold all N permits, then verify the (N+1)th checkout blocks.
    #[tokio::test]
    async fn capacity_caps_open_connections() {
        let dir = TempDir::new().unwrap();
        init_beads(dir.path());
        let capacity = 3usize;
        let pool = ReaderPool::new(dir.path().to_path_buf(), capacity);

        let mut guards = Vec::new();
        for _ in 0..capacity {
            guards.push(pool.checkout().await.expect("checkout"));
        }

        let waiter = pool.checkout();
        tokio::pin!(waiter);

        let blocked = tokio::time::timeout(Duration::from_millis(50), &mut waiter).await;
        assert!(blocked.is_err(), "(N+1)th checkout did not block");

        drop(guards.pop());

        let g = tokio::time::timeout(Duration::from_millis(500), &mut waiter)
            .await
            .expect("(N+1)th checkout did not unblock")
            .expect("checkout returned an error");
        let _ = g.storage();

        drop(guards);
        drop(g);
        assert!(pool.free.lock().unwrap().len() <= capacity);
    }
}
