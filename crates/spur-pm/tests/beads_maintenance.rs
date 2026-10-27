//! Behavioral regressions for automatic maintenance. All data is temporary.

use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use spur_pm::beads_crate::{AdapterConfig, BeadsCrateAdapter};
use spur_pm::{IssueCreate, IssueTracker};
use tempfile::TempDir;

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
