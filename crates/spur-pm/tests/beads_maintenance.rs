//! Behavioral regressions for automatic maintenance. All data is temporary.

use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use spur_pm::beads_crate::{AdapterConfig, BeadsCrateAdapter};
use spur_pm::{IssueCreate, IssueTracker as _};
use tempfile::TempDir;

fn fast_config() -> AdapterConfig {
    AdapterConfig {
        maintenance: spur_pm::beads_crate::maintenance::MaintenancePolicy {
            flush_interval: Duration::from_millis(20),
            sweep_interval: Duration::from_millis(20),
            history_max_bytes: 256,
            wal_truncate_threshold_bytes: 1,
            ..Default::default()
        },
        stale_tmp_min_age: Duration::ZERO,
        ..Default::default()
    }
}

fn seed_history(root: &Path, count: usize, bytes: u64) -> Vec<std::path::PathBuf> {
    let history = root.join(".br_history");
    fs::create_dir_all(&history).unwrap();
    (0..count)
        .map(|i| {
            let path = history.join(format!("issues.20261027_0000{i:02}.jsonl"));
            File::create(&path).unwrap().set_len(bytes).unwrap();
            path
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn automatic_flush_exports_without_reopening_adapter() {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let issue = adapter
        .create_issue(IssueCreate {
            title: "scheduled export witness".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let exported = tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            if fs::read_to_string(dir.path().join("issues.jsonl"))
                .is_ok_and(|text| text.contains(&issue))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        exported.is_ok(),
        "dirty issue must export without reopen or explicit auto_flush"
    );
    assert_eq!(
        adapter
            .read(|s| Ok(s.get_dirty_issue_count()?))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_maintenance_bounds_history_without_dirty_issues() {
    let dir = TempDir::new().unwrap();
    let paths = seed_history(dir.path(), 4, 100 * 1024 * 1024);
    fs::write(
        dir.path().join(".br_history/other.20261027_000000.jsonl"),
        "keep",
    )
    .unwrap();
    fs::create_dir(dir.path().join("quarantine")).unwrap();
    fs::write(dir.path().join("quarantine/recovery.db"), "recovery").unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let pruned = tokio::time::timeout(Duration::from_secs(2), async {
        while paths[0].exists() || paths[1].exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        pruned.is_ok(),
        "startup must prune excess old history even on a clean database"
    );
    assert!(paths[2].exists() && paths[3].exists());
    assert!(dir
        .path()
        .join(".br_history/other.20261027_000000.jsonl")
        .exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("quarantine/recovery.db")).unwrap(),
        "recovery"
    );
    assert_eq!(adapter.read(|s| Ok(s.count_issues()?)).await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_recent_backups_are_preserved() {
    let dir = TempDir::new().unwrap();
    let paths = seed_history(dir.path(), 2, 500 * 1024 * 1024);
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(paths
        .iter()
        .all(|path| path.metadata().unwrap().len() == 500 * 1024 * 1024));
    drop(adapter);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn history_directory_symlink_never_prunes_external_files() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let paths = seed_history(outside.path(), 4, 100 * 1024 * 1024);
    std::os::unix::fs::symlink(
        outside.path().join(".br_history"),
        dir.path().join(".br_history"),
    )
    .unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(paths.iter().all(|path| path.exists()));
    drop(adapter);
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_never_flushes_or_removes_files() {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    // Drain startup housekeeping before seeding dry-run-only changes.
    adapter.maintain_now(false).await.unwrap();
    let paths = seed_history(dir.path(), 4, 100 * 1024 * 1024);
    adapter
        .create_issue(IssueCreate {
            title: "dry run".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let report = adapter.maintain_now(true).await.unwrap();
    assert!(report.dry_run && !report.flushed);
    assert_eq!(report.history.unwrap().removed_files, 2);
    assert!(paths.iter().all(|p| p.exists()));
    assert_eq!(
        adapter
            .read(|s| Ok(s.get_dirty_issue_count()?))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flush_only_report_preserves_last_measured_usage() {
    let dir = TempDir::new().unwrap();
    let mut config = fast_config();
    config.maintenance.sweep_interval = Duration::from_secs(900);
    let adapter = BeadsCrateAdapter::open(dir.path(), config).await.unwrap();
    let measured = adapter.maintain_now(false).await.unwrap();
    assert!(measured.usage.database_bytes > 0);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let report = adapter.maintenance_report().unwrap();
            if report.last_run_epoch_ms > measured.last_run_epoch_ms && report.history.is_none() {
                assert_eq!(report.usage.database_bytes, measured.usage.database_bytes);
                assert_eq!(
                    report.usage_measured_epoch_ms,
                    measured.usage_measured_epoch_ms
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn maintenance_skips_held_flock_and_keeps_actor_available() {
    use fs2::FileExt as _;
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), fast_config())
        .await
        .unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join(".write.lock"))
        .unwrap();
    // Initialization may still be completing an asynchronous sweep.
    tokio::task::spawn_blocking(move || {
        lock.lock_exclusive().unwrap();
        lock
    })
    .await
    .map(|lock| async move {
        let report = tokio::time::timeout(Duration::from_secs(2), adapter.maintain_now(false))
            .await
            .unwrap()
            .unwrap();
        assert!(report.skipped_busy);
        assert_eq!(adapter.read(|s| Ok(s.count_issues()?)).await.unwrap(), 0);
        drop(lock);
        adapter
            .create_issue(IssueCreate {
                title: "after contention".into(),
                ..Default::default()
            })
            .await
            .unwrap();
    })
    .unwrap()
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn continuous_jobs_do_not_starve_flush_or_temporary_cleanup() {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), fast_config())
        .await
        .unwrap();
    let id = adapter
        .create_issue(IssueCreate {
            title: "busy writer".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let temp = dir.path().join("issues.jsonl.12345.tmp");
    fs::write(&temp, "stale").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        adapter.write(|_| Ok(())).await.unwrap();
        if !temp.exists()
            && fs::read_to_string(dir.path().join("issues.jsonl")).is_ok_and(|s| s.contains(&id))
        {
            return;
        }
    }
    panic!("continuous writer jobs starved maintenance deadlines");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_export_preserves_dirty_state_and_retry_succeeds() {
    let dir = TempDir::new().unwrap();
    let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    adapter.maintain_now(false).await.unwrap();
    let id = adapter
        .create_issue(IssueCreate {
            title: "retry export".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let export = dir.path().join("issues.jsonl");
    if export.exists() {
        fs::remove_file(&export).unwrap();
    }
    fs::create_dir(&export).unwrap();
    let paths = seed_history(dir.path(), 4, 100 * 1024 * 1024);
    let failed = adapter.maintain_now(false).await.unwrap();
    assert!(!failed.errors.is_empty() && !failed.flushed);
    assert!(paths.iter().all(|p| p.exists()));
    assert_eq!(
        adapter
            .read(|s| Ok(s.get_dirty_issue_count()?))
            .await
            .unwrap(),
        1
    );
    fs::remove_dir(&export).unwrap();
    tokio::time::timeout(Duration::from_secs(35), async {
        while !fs::read_to_string(&export).is_ok_and(|s| s.contains(&id)) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("scheduled export must retry after the filesystem recovers");
    assert_eq!(
        adapter
            .read(|s| Ok(s.get_dirty_issue_count()?))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wal_truncation_retries_after_pinned_reader_releases() {
    let dir = TempDir::new().unwrap();
    let mut config = AdapterConfig::default();
    config.maintenance.wal_truncate_threshold_bytes = 1;
    let adapter = BeadsCrateAdapter::open(dir.path(), config).await.unwrap();
    adapter.maintain_now(false).await.unwrap();
    let reader = rusqlite::Connection::open(dir.path().join("beads.db")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM issues;")
        .unwrap();
    let id = adapter
        .create_issue(IssueCreate {
            title: "pinned WAL".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let busy = adapter.maintain_now(false).await.unwrap();
    assert!(busy.wal.as_ref().is_some_and(|wal| wal.busy != 0));
    assert!(busy.usage.wal_bytes > 0);
    reader.execute_batch("ROLLBACK").unwrap();
    let drained = adapter.maintain_now(false).await.unwrap();
    assert_eq!(drained.usage.wal_bytes, 0);
    assert_eq!(adapter.get_issue(&id).await.unwrap().title, "pinned WAL");
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_or_future_shared_stamp_cannot_suppress_cleanup() {
    for stamp in [vec![0xff], br#"{"version":1,"completed_epoch_ms":18446744073709551615,"history_max_bytes":256,"history_keep_newest":2,"stale_tmp_min_age":{"secs":0,"nanos":0},"wal_threshold":1}"#.to_vec()] {
        let dir = TempDir::new().unwrap();
        let paths = seed_history(dir.path(), 4, 100);
        fs::write(dir.path().join(".spur-maintenance.json"), stamp).unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), fast_config()).await.unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), async {
            while paths[0].exists() || paths[1].exists() { tokio::time::sleep(Duration::from_millis(20)).await; }
        }).await;
        assert!(outcome.is_ok(), "bad stamp suppressed cleanup: {:?}", adapter.maintenance_report());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_stamp_throttles_second_adapter_startup_sweep() {
    let dir = TempDir::new().unwrap();
    let first = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let successful = first.maintain_now(false).await.unwrap();
    assert!(successful.last_sweep_epoch_ms.is_some());
    let second = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
        .await
        .unwrap();
    let skipped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if second
                .maintenance_report()
                .is_some_and(|report| report.skipped_recent_sweep)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(skipped.is_ok());
}
