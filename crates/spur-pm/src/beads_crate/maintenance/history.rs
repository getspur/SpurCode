use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context;
use chrono::NaiveDateTime;

use super::{HistoryCleanupReport, MaintenancePolicy};
use crate::beads_crate::write_lock::{self, WriteLockAttempt};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device_inode: (u64, u64),
    #[cfg(not(unix))]
    created: SystemTime,
}

impl Identity {
    fn read(meta: &Metadata) -> io::Result<Self> {
        Ok(Self {
            bytes: meta.len(),
            modified: meta.modified()?,
            #[cfg(unix)]
            device_inode: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino())
            },
            #[cfg(not(unix))]
            created: meta.created()?,
        })
    }

    fn same_directory(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.device_inode == other.device_inode
        }
        #[cfg(not(unix))]
        {
            self.created == other.created
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    identity: Identity,
}

/// Immutable deletion plan. Reports project the result; execution revalidates identities.
#[derive(Debug, Clone)]
pub struct HistoryPlan {
    beads_dir: PathBuf,
    policy: MaintenancePolicy,
    directory: Option<Identity>,
    entries: Vec<Entry>,
    candidates: Vec<String>,
    report: HistoryCleanupReport,
}

impl HistoryPlan {
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }
    pub fn report(&self) -> &HistoryCleanupReport {
        &self.report
    }
}

pub(super) fn validate_history_directory(beads_dir: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(beads_dir.join(".br_history")) {
        Ok(meta) => anyhow::ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "history directory must be a real directory"
        ),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn recognized(name: &str) -> bool {
    let Some(stamp) = name
        .strip_prefix("issues.")
        .and_then(|n| n.strip_suffix(".jsonl"))
    else {
        return false;
    };
    stamp.len() == 15
        && NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S")
            .is_ok_and(|date| date.format("%Y%m%d_%H%M%S").to_string() == stamp)
}

/// Inspect exact regular backup names without modifying files or taking a write lock.
pub fn plan_history_cleanup(
    beads_dir: &Path,
    policy: &MaintenancePolicy,
) -> anyhow::Result<HistoryPlan> {
    policy.validate()?;
    validate_history_directory(beads_dir)?;
    let root = beads_dir.join(".br_history");
    let mut plan = HistoryPlan {
        beads_dir: beads_dir.to_path_buf(),
        policy: policy.clone(),
        directory: None,
        entries: Vec::new(),
        candidates: Vec::new(),
        report: HistoryCleanupReport::default(),
    };
    let meta = match fs::symlink_metadata(&root) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(plan),
        Err(err) => return Err(err.into()),
    };
    anyhow::ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "history directory changed during planning"
    );
    plan.directory = Some(Identity::read(&meta)?);
    for item in fs::read_dir(&root)? {
        let item = item?;
        let Some(name) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !recognized(&name) {
            continue;
        }
        let meta = match fs::symlink_metadata(item.path()) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            continue;
        }
        let identity = Identity::read(&meta)?;
        plan.report.bytes_before = plan
            .report
            .bytes_before
            .checked_add(identity.bytes)
            .context("history byte count overflow")?;
        plan.entries.push(Entry { name, identity });
    }
    // Strict fixed-width timestamps sort chronologically; newest entries are protected.
    plan.entries.sort_by(|a, b| b.name.cmp(&a.name));
    plan.report.files_before = plan.entries.len() as u64;
    plan.report.bytes_after = plan.report.bytes_before;
    plan.report.protected_bytes = plan
        .entries
        .iter()
        .take(policy.history_keep_newest)
        .map(|e| e.identity.bytes)
        .sum();
    for entry in plan.entries.iter().skip(policy.history_keep_newest).rev() {
        if plan.report.bytes_after <= policy.history_max_bytes {
            break;
        }
        plan.candidates.push(entry.name.clone());
        plan.report.bytes_after -= entry.identity.bytes;
        plan.report.removed_files += 1;
        plan.report.removed_bytes += entry.identity.bytes;
    }
    plan.report.over_budget_bytes = plan
        .report
        .bytes_after
        .saturating_sub(policy.history_max_bytes);
    Ok(plan)
}

/// Execute a previously inspected plan under a nonblocking cross-process lock.
pub fn execute_history_cleanup(plan: &HistoryPlan) -> anyhow::Result<HistoryCleanupReport> {
    let WriteLockAttempt::Acquired(_guard) =
        write_lock::try_blocking_write_lock_once(&plan.beads_dir)?
    else {
        anyhow::bail!("beads maintenance is busy");
    };
    execute_locked(plan)
}

pub(super) fn execute_locked(plan: &HistoryPlan) -> anyhow::Result<HistoryCleanupReport> {
    let mut report = plan.report.clone();
    report.removed_files = 0;
    report.removed_bytes = 0;
    if plan.candidates.is_empty() {
        return Ok(report);
    }
    let root = plan.beads_dir.join(".br_history");
    validate_history_directory(&plan.beads_dir)?;
    let current = Identity::read(&fs::symlink_metadata(&root)?)?;
    anyhow::ensure!(
        plan.directory
            .as_ref()
            .is_some_and(|old| old.same_directory(&current)),
        "history directory was replaced; replan cleanup"
    );
    #[cfg(unix)]
    let directory = {
        use rustix::fs::{open, Mode, OFlags};
        let file = File::from(open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        anyhow::ensure!(
            current.same_directory(&Identity::read(&file.metadata()?)?),
            "history directory changed while opening"
        );
        file
    };
    // Validate every existing entry before deleting any. Missing candidates are idempotent;
    // missing protected entries invalidate the plan so rollback copies are not exhausted.
    for entry in &plan.entries {
        let candidate = plan.candidates.contains(&entry.name);
        match fs::symlink_metadata(root.join(&entry.name)) {
            Ok(meta) => anyhow::ensure!(
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && Identity::read(&meta)? == entry.identity,
                "history entry {} changed; replan cleanup",
                entry.name
            ),
            Err(err) if candidate && err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    for name in &plan.candidates {
        let entry = plan
            .entries
            .iter()
            .find(|e| &e.name == name)
            .context("invalid history plan")?;
        #[cfg(unix)]
        let result = remove_verified_at(&directory, entry);
        #[cfg(not(unix))]
        let result = remove_verified_path(&root, &current, entry);
        match result {
            Ok(true) => {
                report.removed_files += 1;
                report.removed_bytes += entry.identity.bytes;
            }
            Ok(false) => {}
            Err(err) => report.errors.push(format!("{name}: {err:#}")),
        }
    }
    let remaining = plan_history_cleanup(&plan.beads_dir, &plan.policy)?;
    report.bytes_after = remaining.report.bytes_before;
    report.protected_bytes = remaining.report.protected_bytes;
    report.over_budget_bytes = report
        .bytes_after
        .saturating_sub(plan.policy.history_max_bytes);
    Ok(report)
}

#[cfg(unix)]
fn remove_verified_at(directory: &File, entry: &Entry) -> anyhow::Result<bool> {
    use rustix::fs::{openat, unlinkat, AtFlags, Mode, OFlags};
    let fd = match openat(
        directory,
        entry.name.as_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let file = File::from(fd);
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file() && Identity::read(&meta)? == entry.identity,
        "history entry changed before deletion"
    );
    match unlinkat(directory, entry.name.as_str(), AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(not(unix))]
fn remove_verified_path(root: &Path, directory: &Identity, entry: &Entry) -> anyhow::Result<bool> {
    let meta = fs::symlink_metadata(root)?;
    anyhow::ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && directory.same_directory(&Identity::read(&meta)?),
        "history directory changed"
    );
    let path = root.join(&entry.name);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    anyhow::ensure!(
        meta.is_file()
            && !meta.file_type().is_symlink()
            && Identity::read(&meta)? == entry.identity,
        "history entry changed before deletion"
    );
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}
