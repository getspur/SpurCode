//! Contention and operation metrics for the beads crate adapter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct ContentionMetrics {
    pub lock_wait_total_us: AtomicU64,
    pub lock_busy_total: AtomicU64,
    pub lock_ceiling_total: AtomicU64,
    pub write_total: AtomicU64,
    pub write_error_total: AtomicU64,
    pub read_total: AtomicU64,
    pub sqlite_open_total: AtomicU64,
    pub checkpoint_total: AtomicU64,
    pub conflict_total: AtomicU64,
    pub conflict_exhausted_total: AtomicU64,
    pub auto_flush_skipped_total: AtomicU64,
    pub auto_flush_dirty_total: AtomicU64,
    pub auto_flush_success_total: AtomicU64,
    pub tmp_sweep_removed_total: AtomicU64,
    pub maintenance_total: AtomicU64,
    pub maintenance_busy_total: AtomicU64,
    pub maintenance_error_total: AtomicU64,
    pub history_removed_bytes_total: AtomicU64,
    pub history_removed_files_total: AtomicU64,
}

impl ContentionMetrics {
    pub fn record_lock_wait(&self, d: Duration) {
        self.lock_wait_total_us
            .fetch_add(d.as_micros() as u64, Ordering::Relaxed);
    }
    pub fn incr_busy(&self) {
        self.lock_busy_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_ceiling(&self) {
        self.lock_ceiling_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_write(&self) {
        self.write_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_write_error(&self) {
        self.write_error_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_read(&self) {
        self.read_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_sqlite_open(&self) {
        self.sqlite_open_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_checkpoint(&self) {
        self.checkpoint_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_conflict(&self) {
        self.conflict_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_conflict_exhausted(&self) {
        self.conflict_exhausted_total
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_auto_flush_skipped(&self) {
        self.auto_flush_skipped_total
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_auto_flush_dirty(&self) {
        self.auto_flush_dirty_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_auto_flush_success(&self) {
        self.auto_flush_success_total
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn add_tmp_sweep_removed(&self, n: u64) {
        self.tmp_sweep_removed_total.fetch_add(n, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment() {
        let m = ContentionMetrics::default();
        m.incr_write();
        m.incr_write();
        m.incr_read();
        m.incr_sqlite_open();
        m.incr_checkpoint();
        assert_eq!(m.write_total.load(Ordering::Relaxed), 2);
        assert_eq!(m.read_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.sqlite_open_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.checkpoint_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn lock_wait_accumulates_microseconds() {
        let m = ContentionMetrics::default();
        m.record_lock_wait(Duration::from_millis(5));
        m.record_lock_wait(Duration::from_millis(3));
        assert_eq!(m.lock_wait_total_us.load(Ordering::Relaxed), 8_000);
    }
}
