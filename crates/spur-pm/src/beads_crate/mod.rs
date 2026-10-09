//! Direct-linkage adapter to the pinned `beads_rust` 0.1.15 crate.
//!
//! See `docs/superpowers/specs/2026-05-05-beads_rust-direct-crate-dep-design.md`
//! for the full design.

pub mod adapter;
pub mod backoff;
mod beads_advanced;
mod beads_db;
pub(crate) mod dependency_compat;
mod hygiene;
pub mod init;
pub(crate) mod issue_changes;
pub mod issue_reads;
pub mod issue_tracker;
pub mod metrics;
pub mod reader_pool;
pub mod snapshot;
mod sort_index;
pub(crate) mod summary_query;
pub(crate) mod wal_checkpoint;
pub(crate) mod write_lock;

pub use adapter::{AdapterConfig, BeadsCrateAdapter};

#[cfg(feature = "test-helpers")]
pub mod test_helpers {
    use std::path::Path;
    use std::time::Duration;

    #[allow(dead_code)]
    pub struct TestWriteLockGuard(pub(crate) super::write_lock::WriteLockGuard);

    pub fn blocking_write_lock_with_timeout(
        beads_dir: &Path,
        lock_timeout_ms: Option<u64>,
    ) -> anyhow::Result<TestWriteLockGuard> {
        super::write_lock::blocking_write_lock_with_timeout(beads_dir, lock_timeout_ms)
            .map(TestWriteLockGuard)
    }

    pub fn lock_holder_pid(lock_path: &Path) -> Option<u32> {
        super::write_lock::read_holder_payload(lock_path).and_then(|holder| holder.pid)
    }

    pub fn sweep_stale_jsonl_temps(beads_dir: &Path, min_age: Duration) -> std::io::Result<u64> {
        super::init::sweep_stale_jsonl_temps(beads_dir, min_age)
    }

    pub fn checkpoint_wal_truncate_best_effort(db_path: &Path) {
        super::wal_checkpoint::checkpoint_wal_truncate_best_effort(db_path);
    }
}

#[cfg(test)]
mod issue_reads_tests;
