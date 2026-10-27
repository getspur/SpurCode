use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use beads_rust::storage::sqlite::SqliteStorage;
use serde::{Deserialize, Serialize};

use super::{history, MaintenancePolicy, MaintenanceReport, StorageUsage, WalReport};
use crate::beads_crate::{init, metrics::ContentionMetrics, write_lock};

const STATE_FILE: &str = ".spur-maintenance.json";

pub(crate) type SharedReport = Arc<Mutex<Option<MaintenanceReport>>>;

pub(crate) struct MaintenanceRuntime {
    beads_dir: PathBuf,
    policy: MaintenancePolicy,
    stale_tmp_min_age: Duration,
    metrics: Arc<ContentionMetrics>,
    report: SharedReport,
    next_flush: Instant,
    next_sweep: Instant,
    last_export: Option<u64>,
    last_sweep: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct SweepStamp {
    version: u8,
    completed_epoch_ms: u64,
    history_max_bytes: u64,
    history_keep_newest: usize,
    stale_tmp_min_age: Duration,
    wal_threshold: u64,
}

impl MaintenanceRuntime {
    pub(crate) fn new(
        beads_dir: PathBuf,
        policy: MaintenancePolicy,
        stale_tmp_min_age: Duration,
        metrics: Arc<ContentionMetrics>,
        report: SharedReport,
    ) -> Self {
        let now = Instant::now();
        Self {
            beads_dir,
            next_flush: now + policy.flush_interval,
            next_sweep: now,
            policy,
            stale_tmp_min_age,
            metrics,
            report,
            last_export: None,
            last_sweep: None,
        }
    }

    pub(crate) fn until_due(&self) -> Duration {
        self.next_flush
            .min(self.next_sweep)
            .saturating_duration_since(Instant::now())
    }

    pub(crate) fn tick(&mut self, storage: &mut SqliteStorage, checkpoint: &rusqlite::Connection) {
        let now = Instant::now();
        let sweep = now >= self.next_sweep;
        if now < self.next_flush && !sweep {
            return;
        }
        let report = self.run(storage, checkpoint, false, false, sweep);
        let now = Instant::now();
        self.next_flush = now + self.policy.flush_interval;
        if sweep {
            // Busy/error retries use the flush cadence, avoiding a tight actor loop.
            let delay = if report.skipped_busy || !report.errors.is_empty() {
                self.policy.flush_interval.min(self.policy.sweep_interval)
            } else {
                self.policy.sweep_interval
            };
            self.next_sweep = now + delay;
        }
    }

    pub(crate) fn run_now(
        &mut self,
        storage: &mut SqliteStorage,
        checkpoint: &rusqlite::Connection,
        dry_run: bool,
    ) -> MaintenanceReport {
        self.run(storage, checkpoint, dry_run, true, true)
    }

    fn run(
        &mut self,
        storage: &mut SqliteStorage,
        checkpoint: &rusqlite::Connection,
        dry_run: bool,
        force: bool,
        sweep: bool,
    ) -> MaintenanceReport {
        let started = Instant::now();
        let mut report = MaintenanceReport {
            dry_run,
            last_run_epoch_ms: epoch_ms(),
            last_export_epoch_ms: self.last_export,
            last_sweep_epoch_ms: self.last_sweep,
            ..Default::default()
        };
        if let Some(previous) = self
            .report
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            report.usage = previous.usage.clone();
            report.usage_measured_epoch_ms = previous.usage_measured_epoch_ms;
        }
        let result = self.run_locked(storage, checkpoint, force, sweep, &mut report);
        if let Err(err) = result {
            report.errors.push(format!("{err:#}"));
        }
        report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !dry_run {
            self.metrics
                .maintenance_total
                .fetch_add(1, Ordering::Relaxed);
            if report.skipped_busy {
                self.metrics
                    .maintenance_busy_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.metrics
                .maintenance_error_total
                .fetch_add(report.errors.len() as u64, Ordering::Relaxed);
            if let Some(history) = &report.history {
                self.metrics
                    .history_removed_bytes_total
                    .fetch_add(history.removed_bytes, Ordering::Relaxed);
                self.metrics
                    .history_removed_files_total
                    .fetch_add(history.removed_files, Ordering::Relaxed);
            }
            if !report.errors.is_empty() {
                tracing::warn!(errors = ?report.errors, "beads maintenance failed; will retry");
            }
            *self.report.lock().unwrap_or_else(|err| err.into_inner()) = Some(report.clone());
        }
        report
    }

    fn run_locked(
        &mut self,
        storage: &mut SqliteStorage,
        checkpoint: &rusqlite::Connection,
        force: bool,
        sweep: bool,
        report: &mut MaintenanceReport,
    ) -> anyhow::Result<()> {
        // Foreground callers hold this flock before queueing jobs. Never wait here.
        let write_lock::WriteLockAttempt::Acquired(_guard) =
            write_lock::try_blocking_write_lock_once(&self.beads_dir)?
        else {
            report.skipped_busy = true;
            return Ok(());
        };
        report.dirty_issues = storage.get_dirty_issue_count()?;
        if !report.dry_run {
            let flushed = safe_auto_flush(storage, &self.beads_dir)?;
            report.flushed = flushed.flushed;
            if flushed.flushed {
                self.metrics.incr_auto_flush_dirty();
                self.metrics.incr_auto_flush_success();
                self.last_export = Some(epoch_ms());
                report.last_export_epoch_ms = self.last_export;
            } else {
                self.metrics.incr_auto_flush_skipped();
            }
        }
        if !sweep {
            return Ok(());
        }
        if !force && self.recent_sweep()? {
            report.skipped_recent_sweep = true;
            report.last_sweep_epoch_ms = self.last_sweep;
            return Ok(());
        }
        // Export errors returned above preserve every pre-export recovery copy.
        let plan = history::plan_history_cleanup(&self.beads_dir, &self.policy)?;
        let cleanup = if report.dry_run {
            plan.report().clone()
        } else {
            history::execute_locked(&plan)?
        };
        report.errors.extend(cleanup.errors.iter().cloned());
        report.history = Some(cleanup);
        if !report.dry_run {
            report.temporary_files_removed =
                init::sweep_stale_jsonl_temps(&self.beads_dir, self.stale_tmp_min_age)?;
            self.metrics
                .add_tmp_sweep_removed(report.temporary_files_removed);
            report.wal = truncate_if_large(
                checkpoint,
                &self.beads_dir,
                self.policy.wal_truncate_threshold_bytes,
            )?;
        }
        report.usage = storage_usage(&self.beads_dir)?;
        report.usage_measured_epoch_ms = Some(epoch_ms());
        if !report.dry_run && report.errors.is_empty() {
            let completed = epoch_ms();
            self.save_stamp(completed)?;
            self.last_sweep = Some(completed);
            report.last_sweep_epoch_ms = self.last_sweep;
        }
        Ok(())
    }

    fn recent_sweep(&mut self) -> anyhow::Result<bool> {
        let path = self.beads_dir.join(STATE_FILE);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        // Malformed/future stamps cannot suppress cleanup. Bound untrusted reads to one page.
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 4096 {
            return Ok(false);
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(4096).read_to_end(&mut bytes)?;
        let Ok(stamp) = serde_json::from_slice::<SweepStamp>(&bytes) else {
            return Ok(false);
        };
        let Some(age) = epoch_ms().checked_sub(stamp.completed_epoch_ms) else {
            return Ok(false);
        };
        let same_policy = stamp.version == 1
            && stamp.history_max_bytes == self.policy.history_max_bytes
            && stamp.history_keep_newest == self.policy.history_keep_newest
            && stamp.stale_tmp_min_age == self.stale_tmp_min_age
            && stamp.wal_threshold == self.policy.wal_truncate_threshold_bytes;
        if same_policy && Duration::from_millis(age) < self.policy.sweep_interval {
            self.last_sweep = Some(stamp.completed_epoch_ms);
            return Ok(true);
        }
        Ok(false)
    }

    fn save_stamp(&self, completed_epoch_ms: u64) -> anyhow::Result<()> {
        let stamp = SweepStamp {
            version: 1,
            completed_epoch_ms,
            history_max_bytes: self.policy.history_max_bytes,
            history_keep_newest: self.policy.history_keep_newest,
            stale_tmp_min_age: self.stale_tmp_min_age,
            wal_threshold: self.policy.wal_truncate_threshold_bytes,
        };
        let mut temp = tempfile::NamedTempFile::new_in(&self.beads_dir)?;
        serde_json::to_writer(temp.as_file_mut(), &stamp)?;
        temp.as_file().sync_all()?;
        temp.persist(self.beads_dir.join(STATE_FILE))?;
        Ok(())
    }
}

/// Keep the pinned exporter away from history-directory/file symlinks.
pub(crate) fn safe_auto_flush(
    storage: &mut SqliteStorage,
    beads_dir: &Path,
) -> anyhow::Result<beads_rust::sync::AutoFlushResult> {
    if storage.get_dirty_issue_count()? > 0 {
        history::validate_history_directory(beads_dir)?;
        let root = beads_dir.join(".br_history");
        if root.exists() {
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with("issues.") {
                    anyhow::ensure!(
                        !entry.file_type()?.is_symlink(),
                        "history backup symlink prevents safe export"
                    );
                }
            }
        }
    }
    Ok(beads_rust::sync::auto_flush(storage, beads_dir)?)
}

fn truncate_if_large(
    conn: &rusqlite::Connection,
    root: &Path,
    threshold: u64,
) -> anyhow::Result<Option<WalReport>> {
    let path = root.join("beads.db-wal");
    let before = file_bytes(&path)?;
    if before <= threshold {
        return Ok(None);
    }
    let (busy, log_frames, checkpointed_frames) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
    }
    Ok(Some(WalReport {
        bytes_before: before,
        bytes_after: file_bytes(&path)?,
        busy,
        log_frames,
        checkpointed_frames,
    }))
}

fn file_bytes(path: &Path) -> io::Result<u64> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(meta.len()),
        Ok(_) => Ok(0),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

fn tree_usage(root: &Path) -> io::Result<(u64, u64)> {
    let mut pending = vec![root.to_path_buf()];
    let (mut bytes, mut files) = (0_u64, 0_u64);
    while let Some(path) = pending.pop() {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_file() {
            bytes = bytes.saturating_add(meta.len());
            files += 1;
        } else if meta.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok((bytes, files))
}

fn storage_usage(root: &Path) -> io::Result<StorageUsage> {
    let (quarantine_bytes, quarantine_files) = tree_usage(&root.join("quarantine"))?;
    Ok(StorageUsage {
        database_bytes: file_bytes(&root.join("beads.db"))?,
        export_bytes: file_bytes(&root.join("issues.jsonl"))?,
        wal_bytes: file_bytes(&root.join("beads.db-wal"))?,
        history_bytes: tree_usage(&root.join(".br_history"))?.0,
        quarantine_bytes,
        quarantine_files,
    })
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| d.as_millis().try_into().ok())
        .unwrap_or(0)
}
