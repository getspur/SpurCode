use super::*;
use std::fs::{self, File};
use tempfile::TempDir;

fn fixture(sizes: &[u64]) -> (TempDir, MaintenancePolicy) {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".br_history")).unwrap();
    for (n, size) in sizes.iter().enumerate() {
        File::create(
            dir.path()
                .join(format!(".br_history/issues.20261027_0000{n:02}.jsonl")),
        )
        .unwrap()
        .set_len(*size)
        .unwrap();
    }
    (
        dir,
        MaintenancePolicy {
            history_max_bytes: 256,
            ..Default::default()
        },
    )
}

#[test]
fn dry_run_and_execution_agree_and_preserve_newest_two() {
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    assert_eq!(
        plan.candidates(),
        &[
            "issues.20261027_000000.jsonl",
            "issues.20261027_000001.jsonl"
        ]
    );
    assert_eq!(plan.report().bytes_before, 400);
    assert_eq!(plan.report().bytes_after, 200);
    assert_eq!(
        fs::read_dir(dir.path().join(".br_history"))
            .unwrap()
            .count(),
        4
    );
    let report = execute_history_cleanup(&plan).unwrap();
    assert_eq!(report.removed_bytes, 200);
    assert_eq!(report.removed_files, 2);
    assert_eq!(report.bytes_after, 200);
    let again = plan_history_cleanup(dir.path(), &policy).unwrap();
    assert!(again.candidates().is_empty());
}

#[test]
fn protected_copies_can_exceed_soft_budget() {
    let (dir, policy) = fixture(&[100, 500, 500]);
    let report =
        execute_history_cleanup(&plan_history_cleanup(dir.path(), &policy).unwrap()).unwrap();
    assert_eq!(report.removed_bytes, 100);
    assert_eq!(report.bytes_after, 1000);
    assert_eq!(report.over_budget_bytes, 744);
}

#[test]
fn unknown_names_and_directories_are_not_candidates() {
    let (dir, policy) = fixture(&[100, 100, 100]);
    let history = dir.path().join(".br_history");
    for name in [
        "issues.20260230_000000.jsonl",
        "issues.20261027_000000.jsonl.bak",
        "other.20261027_000000.jsonl",
        "issues.20261027_000000xjsonl",
    ] {
        fs::write(history.join(name), "preserve").unwrap();
    }
    fs::create_dir(history.join("issues.20261027_000005.jsonl")).unwrap();
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    assert_eq!(plan.report().files_before, 3);
    execute_history_cleanup(&plan).unwrap();
    assert!(history.join("issues.20260230_000000.jsonl").exists());
    assert!(history.join("issues.20261027_000005.jsonl").is_dir());
}

#[test]
fn stale_plan_rejects_replaced_candidate_before_deletion() {
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    let path = dir.path().join(".br_history/issues.20261027_000000.jsonl");
    fs::remove_file(&path).unwrap();
    fs::write(&path, "replacement must survive").unwrap();
    assert!(execute_history_cleanup(&plan).is_err());
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "replacement must survive"
    );
}

#[test]
fn losing_protected_copy_invalidates_plan() {
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    fs::remove_file(dir.path().join(".br_history/issues.20261027_000003.jsonl")).unwrap();
    assert!(execute_history_cleanup(&plan).is_err());
    assert!(dir
        .path()
        .join(".br_history/issues.20261027_000000.jsonl")
        .exists());
}

#[cfg(unix)]
#[test]
fn symlinks_and_swapped_history_directory_are_preserved() {
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("target");
    fs::write(&target, "external").unwrap();
    let link = dir.path().join(".br_history/issues.20261027_000005.jsonl");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    assert_eq!(plan.report().files_before, 4);
    fs::rename(
        dir.path().join(".br_history"),
        dir.path().join("old-history"),
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".br_history")).unwrap();
    assert!(execute_history_cleanup(&plan).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "external");
}

#[test]
fn disappeared_candidate_is_idempotent() {
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    fs::remove_file(dir.path().join(".br_history/issues.20261027_000000.jsonl")).unwrap();
    let report = execute_history_cleanup(&plan).unwrap();
    assert_eq!(report.removed_files, 1);
    assert_eq!(report.bytes_after, 200);
}

#[test]
fn invalid_policy_is_rejected_before_creating_files() {
    let dir = TempDir::new().unwrap();
    let mut policy = MaintenancePolicy {
        flush_interval: Duration::ZERO,
        ..Default::default()
    };
    assert!(policy.validate().is_err());
    assert!(plan_history_cleanup(dir.path(), &policy).is_err());
    policy.flush_interval = Duration::from_millis(20);
    policy.history_keep_newest = 0;
    assert!(policy.validate().is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn permission_denied_deletions_are_reported_and_retryable() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, policy) = fixture(&[100, 100, 100, 100]);
    let history = dir.path().join(".br_history");
    let original = fs::metadata(&history).unwrap().permissions();
    let plan = plan_history_cleanup(dir.path(), &policy).unwrap();
    fs::set_permissions(&history, fs::Permissions::from_mode(0o555)).unwrap();
    // Root ignores Unix mode permissions; explicitly detect that environment.
    let probe = history.join("permission-probe");
    if File::create(&probe).is_ok() {
        fs::remove_file(probe).unwrap();
        fs::set_permissions(history, original).unwrap();
        return;
    }
    let result = execute_history_cleanup(&plan);
    fs::set_permissions(&history, original).unwrap();
    let report = result.unwrap();
    assert_eq!(report.removed_files, 0);
    assert_eq!(report.bytes_after, 400);
    assert_eq!(report.errors.len(), 2);
    assert_eq!(execute_history_cleanup(&plan).unwrap().removed_files, 2);
}
