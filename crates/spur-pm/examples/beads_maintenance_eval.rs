//! Reproducible, isolated evidence consumed by the maintenance research notebook.
use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context as _};
use serde_json::json;
use spur_pm::beads_crate::maintenance::{
    execute_history_cleanup, plan_history_cleanup, MaintenancePolicy,
};
use spur_pm::beads_crate::{AdapterConfig, BeadsCrateAdapter};
use spur_pm::{IssueCreate, IssueTracker as _};

fn evaluate_history(bytes: u64) -> anyhow::Result<serde_json::Value> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let history = root.join(".br_history");
    fs::create_dir(&history)?;
    fs::create_dir(root.join("quarantine"))?;
    for path in [
        "issues.jsonl",
        "beads.db",
        "quarantine/recovery.db",
        ".br_history/unknown.jsonl",
    ] {
        fs::write(root.join(path), b"preserve this artifact")?;
    }
    let start = chrono::NaiveDateTime::parse_from_str("20261027_000000", "%Y%m%d_%H%M%S")?;
    let mut names = Vec::new();
    for index in 0..100 {
        let timestamp = start + chrono::Duration::seconds(index);
        let name = format!("issues.{}.jsonl", timestamp.format("%Y%m%d_%H%M%S"));
        File::create(history.join(&name))?.set_len(bytes)?;
        names.push(name);
    }
    let policy = MaintenancePolicy::default();
    let plan = plan_history_cleanup(root, &policy)?;
    let dry_run_unchanged = names
        .iter()
        .all(|n| history.join(n).metadata().is_ok_and(|m| m.len() == bytes));
    let planned = plan.report().clone();
    let started = Instant::now();
    let executed = execute_history_cleanup(&plan)?;
    let elapsed_ms = started.elapsed().as_millis();
    let retained = names.iter().filter(|n| history.join(n).exists()).count();
    let protected_survived = names
        .iter()
        .rev()
        .take(policy.history_keep_newest)
        .all(|n| history.join(n).exists());
    let canonical_preserved = [
        "issues.jsonl",
        "beads.db",
        "quarantine/recovery.db",
        ".br_history/unknown.jsonl",
    ]
    .iter()
    .all(|p| fs::read(root.join(p)).is_ok_and(|b| b == b"preserve this artifact"));
    let idempotent =
        execute_history_cleanup(&plan_history_cleanup(root, &policy)?)?.removed_files == 0;
    let expected_retained = (policy.history_max_bytes / bytes)
        .max(policy.history_keep_newest as u64)
        .min(100);
    let passed = dry_run_unchanged
        && protected_survived
        && canonical_preserved
        && idempotent
        && executed.errors.is_empty()
        && planned.removed_bytes == executed.removed_bytes
        && retained as u64 == expected_retained;
    ensure!(passed, "history evaluation failed for {bytes} bytes");
    Ok(json!({
        "backup_bytes": bytes, "files_before": 100, "bytes_before": executed.bytes_before,
        "files_after": retained, "bytes_after": executed.bytes_after,
        "removed_bytes": executed.removed_bytes, "over_budget_bytes": executed.over_budget_bytes,
        "protected_bytes": executed.protected_bytes, "dry_run_unchanged": dry_run_unchanged,
        "protected_survived": protected_survived, "canonical_preserved": canonical_preserved,
        "idempotent": idempotent, "passed": passed, "duration_ms": elapsed_ms
    }))
}

async fn evaluate_runtime() -> anyhow::Result<serde_json::Value> {
    let dir = tempfile::tempdir()?;
    let mut config = AdapterConfig::default();
    config.maintenance.flush_interval = Duration::from_millis(20);
    config.maintenance.wal_truncate_threshold_bytes = 1;
    let adapter = BeadsCrateAdapter::open(dir.path(), config).await?;
    adapter.maintain_now(false).await?;
    let started = Instant::now();
    let id = adapter
        .create_issue(IssueCreate {
            title: "notebook automatic export witness".into(),
            ..Default::default()
        })
        .await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !fs::read_to_string(dir.path().join("issues.jsonl")).is_ok_and(|s| s.contains(&id)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("automatic export deadline")?;
    let export_latency_ms = started.elapsed().as_millis();
    let reader = rusqlite::Connection::open(dir.path().join("beads.db"))?;
    reader.execute_batch("BEGIN; SELECT * FROM issues;")?;
    let second = adapter
        .create_issue(IssueCreate {
            title: "notebook WAL witness".into(),
            ..Default::default()
        })
        .await?;
    let pinned = adapter.maintain_now(false).await?;
    reader.execute_batch("ROLLBACK")?;
    let released = adapter.maintain_now(false).await?;
    let passed = pinned.wal.as_ref().is_some_and(|wal| wal.busy != 0)
        && released.usage.wal_bytes == 0
        && pinned.errors.is_empty()
        && released.errors.is_empty()
        && adapter.get_issue(&second).await?.title == "notebook WAL witness";
    ensure!(passed, "runtime evaluation failed");
    Ok(
        json!({"passed":passed, "export_latency_ms": export_latency_ms, "test_flush_ms":20,
        "pinned_reader":pinned, "released_reader":released}),
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let output = std::env::args()
        .nth(1)
        .context("usage: beads_maintenance_eval <output.json>")?;
    let cases = [10, 50, 100, 500]
        .into_iter()
        .map(|mib| evaluate_history(mib * 1024 * 1024))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let result = json!({"policy":MaintenancePolicy::default(), "history_cases":cases,
        "runtime":evaluate_runtime().await?, "fixture_kind":"temporary sparse files; sizes are logical bytes"});
    fs::write(Path::new(&output), serde_json::to_vec_pretty(&result)?)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
