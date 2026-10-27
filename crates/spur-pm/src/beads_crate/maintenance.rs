//! Bounded housekeeping for redundant beads files; canonical data is never pruned.

use std::time::{Duration, Instant};

use serde::Serialize;

mod history;
pub(crate) mod runtime;

pub use history::{execute_history_cleanup, plan_history_cleanup, HistoryPlan};

/// Approved defaults: coalesced exports, soft history budget, and occasional WAL truncation.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenancePolicy {
    pub flush_interval: Duration,
    pub sweep_interval: Duration,
    pub history_max_bytes: u64,
    pub history_keep_newest: usize,
    pub wal_truncate_threshold_bytes: u64,
}

impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self {
            flush_interval: Duration::from_secs(30),
            sweep_interval: Duration::from_secs(900),
            history_max_bytes: 256 * 1024 * 1024,
            history_keep_newest: 2,
            wal_truncate_threshold_bytes: 64 * 1024 * 1024,
        }
    }
}

impl MaintenancePolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.flush_interval.is_zero(),
            "maintenance flush interval must be positive"
        );
        anyhow::ensure!(
            !self.sweep_interval.is_zero(),
            "maintenance sweep interval must be positive"
        );
        anyhow::ensure!(
            Instant::now().checked_add(self.flush_interval).is_some(),
            "maintenance flush interval is too large"
        );
        anyhow::ensure!(
            Instant::now().checked_add(self.sweep_interval).is_some(),
            "maintenance sweep interval is too large"
        );
        anyhow::ensure!(
            self.history_max_bytes > 0,
            "history byte budget must be positive"
        );
        anyhow::ensure!(
            self.history_keep_newest > 0,
            "at least one history backup must be protected"
        );
        anyhow::ensure!(
            self.wal_truncate_threshold_bytes > 0,
            "WAL threshold must be positive"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HistoryCleanupReport {
    pub files_before: u64,
    pub bytes_before: u64,
    pub removed_files: u64,
    pub removed_bytes: u64,
    pub bytes_after: u64,
    pub protected_bytes: u64,
    pub over_budget_bytes: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StorageUsage {
    pub database_bytes: u64,
    pub export_bytes: u64,
    pub wal_bytes: u64,
    pub history_bytes: u64,
    pub quarantine_bytes: u64,
    pub quarantine_files: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WalReport {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub busy: i64,
    pub log_frames: i64,
    pub checkpointed_frames: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MaintenanceReport {
    pub dry_run: bool,
    pub skipped_busy: bool,
    pub skipped_recent_sweep: bool,
    pub dirty_issues: usize,
    pub flushed: bool,
    pub history: Option<HistoryCleanupReport>,
    pub wal: Option<WalReport>,
    pub temporary_files_removed: u64,
    /// Last measured sizes; flush-only and busy runs retain the previous sample.
    pub usage: StorageUsage,
    pub usage_measured_epoch_ms: Option<u64>,
    pub duration_ms: u64,
    pub last_run_epoch_ms: u64,
    /// Last successful export performed by this actor's maintenance runtime.
    pub last_export_epoch_ms: Option<u64>,
    pub last_sweep_epoch_ms: Option<u64>,
    pub errors: Vec<String>,
}

#[cfg(test)]
#[path = "maintenance/history_tests.rs"]
mod history_tests;
