//! Init-time guards for the beads crate adapter.
//!
//! Each function is a precondition that must hold before `BeadsCrateAdapter`
//! is allowed to open the writer connection.
#![allow(unsafe_code)] // libc::statfs FFI for NFS/SMB/CIFS detection.

use beads_rust::storage::sqlite::SqliteStorage;
use beads_rust::sync;
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::beads_crate::{wal_checkpoint, write_lock};

const SKIP_PROBE_ENV: &str = "SPUR_BEADS_SKIP_PROBE";
const NO_QUARANTINE_ENV: &str = "SPUR_BEADS_NO_QUARANTINE";

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(
        "non-local filesystem detected at {path}: fs_type = {fs_type}. \
             flock semantics are not portable here. \
             Set allow_non_local_fs=true in config to bypass."
    )]
    NonLocalFilesystem { path: String, fs_type: String },
    #[error("pre-open SQLite quick_check failed for {path}: {message}")]
    QuickCheckFailed { path: String, message: String },
}

/// Returns Ok(()) for local filesystems; Err for known network mounts (NFS, SMB, etc.).
/// Best-effort: on platforms where we cannot determine the FS type, returns Ok.
pub fn detect_local_fs(beads_dir: &Path) -> Result<(), InitError> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(path) = CString::new(beads_dir.as_os_str().as_bytes()) else {
            return Ok(()); // path can't be C-stringified — best-effort allow
        };
        // SAFETY: `statfs` is a plain C output struct; zeroing it before the
        // syscall fills the fields is the standard initialization pattern.
        let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is a valid NUL-terminated C string and `buf` points to
        // writable memory for the duration of the call.
        let rc = unsafe { libc::statfs(path.as_ptr(), &mut buf) };
        if rc != 0 {
            return Ok(());
        } // can't determine; allow
          // Magic numbers from <linux/magic.h>; kernel stores them as __u32.
          // On 32-bit Linux, libc::statfs.f_type is i32, and `i32 as u64` SIGN-extends
          // (Rust language reference, Numeric Cast). The CIFS magic 0xFF534D42 has
          // the high bit set, so a direct `as u64` would yield 0xFFFFFFFF_FF534D42
          // and never match the constant. Cast through u32 first to zero-extend.
        const NFS_SUPER_MAGIC: u64 = 0x6969;
        const SMB_SUPER_MAGIC: u64 = 0x517B;
        const CIFS_MAGIC_NUMBER: u64 = 0xFF534D42;
        let ty = buf.f_type as u32 as u64;
        if ty == NFS_SUPER_MAGIC || ty == SMB_SUPER_MAGIC || ty == CIFS_MAGIC_NUMBER {
            return Err(InitError::NonLocalFilesystem {
                path: beads_dir.display().to_string(),
                fs_type: format!("0x{ty:x}"),
            });
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(path) = CString::new(beads_dir.as_os_str().as_bytes()) else {
            return Ok(()); // path can't be C-stringified — best-effort allow
        };
        // SAFETY: `statfs` is a plain C output struct; zeroing it before the
        // syscall fills the fields is the standard initialization pattern.
        let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is a valid NUL-terminated C string and `buf` points to
        // writable memory for the duration of the call.
        let rc = unsafe { libc::statfs(path.as_ptr(), &mut buf) };
        if rc != 0 {
            return Ok(());
        }
        // SAFETY: `statfs` returned success and `f_fstypename` is a
        // NUL-terminated filesystem type name on macOS.
        let fs_name = unsafe {
            let raw = buf.f_fstypename.as_ptr();
            std::ffi::CStr::from_ptr(raw).to_string_lossy().into_owned()
        };
        if matches!(fs_name.as_str(), "nfs" | "smbfs" | "cifs" | "afpfs") {
            return Err(InitError::NonLocalFilesystem {
                path: beads_dir.display().to_string(),
                fs_type: fs_name,
            });
        }
    }
    // Other targets (e.g. windows) have no FS-type probe: best-effort allow.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = beads_dir;
    Ok(())
}

fn quick_check_probe_enabled() -> bool {
    !std::env::var(SKIP_PROBE_ENV).is_ok_and(|value| value == "1")
}

fn pre_open_quick_check(db_path: &Path) -> Result<(), InitError> {
    if !quick_check_probe_enabled() || !db_path.exists() {
        return Ok(());
    }

    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|err| InitError::QuickCheckFailed {
                path: db_path.display().to_string(),
                message: err.to_string(),
            })?;
    let result: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|err| InitError::QuickCheckFailed {
            path: db_path.display().to_string(),
            message: err.to_string(),
        })?;

    if result == "ok" {
        Ok(())
    } else {
        Err(InitError::QuickCheckFailed {
            path: db_path.display().to_string(),
            message: result,
        })
    }
}

#[derive(Debug, Serialize)]
struct QuarantineManifest {
    created_at_epoch_ms: u64,
    entries: Vec<QuarantineEntry>,
}

#[derive(Debug, Serialize)]
struct QuarantineEntry {
    original_path: String,
    quarantined_path: String,
    size: u64,
    mtime_epoch_ms: Option<u64>,
    classifier_match: &'static str,
    reason: &'static str,
}

pub(crate) fn quarantine_stale_siblings(beads_dir: &Path) {
    if std::env::var(NO_QUARANTINE_ENV).is_ok_and(|value| value == "1") {
        return;
    }

    let Ok(entries) = std::fs::read_dir(beads_dir) else {
        return;
    };
    let startup_ts = current_epoch_ms();
    let quarantine_dir = beads_dir
        .join("quarantine")
        .join(format!("{startup_ts}-{}", std::process::id()));
    let mut manifest_entries = Vec::new();

    for entry in entries {
        let Ok(entry) = entry else {
            tracing::warn!(
                beads_dir = %beads_dir.display(),
                "failed to inspect beads directory entry during startup quarantine"
            );
            continue;
        };
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            tracing::warn!(
                path = %path.display(),
                "failed to inspect beads directory entry type during startup quarantine"
            );
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((classifier_match, reason)) = quarantine_classifier(name) else {
            continue;
        };

        if let Err(err) = std::fs::create_dir_all(&quarantine_dir) {
            tracing::warn!(
                error = %err,
                quarantine_dir = %quarantine_dir.display(),
                "failed to create beads startup quarantine directory"
            );
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %path.display(),
                    "failed to stat stale beads sibling before quarantine"
                );
                continue;
            }
        };
        let quarantined_path = quarantine_dir.join(name);
        match std::fs::rename(&path, &quarantined_path) {
            Ok(()) => manifest_entries.push(QuarantineEntry {
                original_path: path.display().to_string(),
                quarantined_path: quarantined_path.display().to_string(),
                size: metadata.len(),
                mtime_epoch_ms: metadata.modified().ok().and_then(system_time_epoch_ms),
                classifier_match,
                reason,
            }),
            Err(err) => tracing::warn!(
                error = %err,
                path = %path.display(),
                quarantine_dir = %quarantine_dir.display(),
                "failed to move stale beads sibling into startup quarantine"
            ),
        }
    }

    if manifest_entries.is_empty() {
        return;
    }

    let manifest = QuarantineManifest {
        created_at_epoch_ms: startup_ts,
        entries: manifest_entries,
    };
    let manifest_path = quarantine_dir.join("quarantine.json");
    match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(&manifest_path, bytes) {
                tracing::warn!(
                    error = %err,
                    manifest_path = %manifest_path.display(),
                    "failed to write beads startup quarantine manifest"
                );
            }
        }
        Err(err) => tracing::warn!(
            error = %err,
            manifest_path = %manifest_path.display(),
            "failed to serialize beads startup quarantine manifest"
        ),
    }
}

fn quarantine_classifier(name: &str) -> Option<(&'static str, &'static str)> {
    if name == "beads.recovered.db" {
        return Some(("beads.recovered.db", "top-level recovered SQLite database"));
    }
    if name == "data.db" {
        return Some(("data.db", "top-level legacy SQLite database"));
    }
    if name.contains("-shm.broken-") {
        return Some((
            "*-shm.broken-*",
            "stale SQLite shared-memory sidecar from prior repair path",
        ));
    }
    if name.contains("-wal.broken-") {
        return Some((
            "*-wal.broken-*",
            "stale SQLite WAL sidecar from prior repair path",
        ));
    }

    const MARKERS: [(&str, &str); 7] = [
        (".broken-", "*.broken-*"),
        (".stale-", "*.stale-*"),
        (".btree-corrupt-", "*.btree-corrupt-*"),
        (".frankenwal-broken-", "*.frankenwal-broken-*"),
        (".pre-vacuum-", "*.pre-vacuum-*"),
        (".pre-13003-fix-", "*.pre-13003-fix-*"),
        (".reserved12-", "*.reserved12-*"),
    ];
    for (marker, pattern) in MARKERS {
        if name.contains(marker) {
            return Some((pattern, "stale DB-like sibling from prior repair path"));
        }
    }
    None
}

fn current_epoch_ms() -> u64 {
    system_time_epoch_ms(SystemTime::now()).unwrap_or(0)
}

fn system_time_epoch_ms(time: SystemTime) -> Option<u64> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    duration.as_millis().try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn local_tempdir_is_local() {
        let dir = TempDir::new().unwrap();
        // tmpfs / APFS local — should pass
        assert!(detect_local_fs(dir.path()).is_ok());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn nul_byte_in_path_does_not_panic() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;
        // Construct a path with an interior NUL byte. CString::new will reject this.
        let bad = PathBuf::from(OsString::from_vec(vec![b'a', 0, b'b']));
        // Best-effort: must not panic. Result is `Ok(())` because we can't determine FS.
        assert!(detect_local_fs(&bad).is_ok());
    }

    /// On 32-bit Linux, `libc::statfs.f_type` is `i32`. The CIFS magic
    /// `0xFF534D42` has the high bit set, so its `i32` representation is
    /// negative. Per the Rust reference, `i32 as u64` SIGN-extends — a direct
    /// cast yields `0xFFFFFFFF_FF534D42` and never matches the magic constant.
    /// Casting through `u32` first zero-extends. This test locks the cast
    /// contract regardless of build target.
    #[test]
    fn cifs_magic_does_not_sign_extend_through_i32() {
        let signed: i32 = 0xFF534D42_u32 as i32;
        assert!(signed.is_negative());
        let direct: u64 = signed as u64;
        let via_u32: u64 = signed as u32 as u64;
        assert_eq!(direct, 0xFFFFFFFF_FF534D42_u64);
        assert_eq!(via_u32, 0xFF534D42_u64);
        assert_ne!(direct, via_u32);
    }
}

/// Pattern matching the temp files `beads_rust` creates during atomic JSONL writes.
/// Per `beads_rust` 0.1.15 `sync::export_temp_path`:
///
/// ```ignore
/// pub(crate) fn export_temp_path(output_path: &Path) -> PathBuf {
///     output_path.with_extension(format!("jsonl.{}.tmp", std::process::id()))
/// }
/// ```
///
/// For input `issues.jsonl`, `Path::with_extension` strips `.jsonl` and appends
/// the new extension, producing `issues.jsonl.<pid>.tmp`. The PID is decimal
/// digits; there is NO random suffix. We deliberately match strictly so we
/// never touch `SQLite` sidecars (`-wal`, `-shm`) or the live `issues.jsonl`.
fn is_jsonl_temp_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("issues.jsonl.") else {
        return false;
    };
    let Some(pid_str) = rest.strip_suffix(".tmp") else {
        return false;
    };
    !pid_str.is_empty() && pid_str.bytes().all(|b| b.is_ascii_digit())
}

/// Sweep stale jsonl temp files older than `min_age`. Returns count removed.
/// Caller MUST hold .write.lock when calling this; we do not acquire it here.
pub(crate) fn sweep_stale_jsonl_temps(beads_dir: &Path, min_age: Duration) -> std::io::Result<u64> {
    let now = SystemTime::now();
    let mut removed = 0;
    let entries = match std::fs::read_dir(beads_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !is_jsonl_temp_file(name_str) {
            continue;
        }
        // TOCTOU: the file may have been renamed/unlinked between read_dir and
        // metadata (e.g., a concurrent atomic write completing). Skip the entry
        // instead of failing the whole sweep.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = now.duration_since(modified) {
                if age >= min_age && std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod sweep_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn removes_temp_left_by_pinned_exporter() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("issues.jsonl");
        // A directory at the destination makes the final rename fail, leaving
        // the real dependency's temp file rather than a hand-written fixture.
        std::fs::create_dir(&output).unwrap();
        let storage = SqliteStorage::open_memory().unwrap();
        let config = sync::ExportConfig {
            force: true,
            ..Default::default()
        };
        assert!(sync::export_to_jsonl(&storage, &output, &config).is_err());
        let orphan = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_file())
            .expect("failed export must leave a temp file");

        let _guard = write_lock::blocking_write_lock_with_timeout(dir.path(), Some(100)).unwrap();
        let min_age = crate::beads_crate::AdapterConfig::default().stale_tmp_min_age;
        assert_eq!(sweep_stale_jsonl_temps(dir.path(), min_age).unwrap(), 0);
        filetime::set_file_mtime(&orphan, filetime::FileTime::from_system_time(UNIX_EPOCH))
            .unwrap();
        assert_eq!(sweep_stale_jsonl_temps(dir.path(), min_age).unwrap(), 1);
        assert!(!orphan.exists());
        assert!(output.is_dir());
    }

    #[test]
    fn ignores_wal_and_shm_sidecars_and_live_jsonl() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("issues.jsonl"), b"x").unwrap();
        std::fs::write(dir.path().join("issues.jsonl-wal"), b"x").unwrap();
        std::fs::write(dir.path().join("issues.jsonl-shm"), b"x").unwrap();
        std::fs::write(dir.path().join("beads.db-wal"), b"x").unwrap();
        let removed = sweep_stale_jsonl_temps(dir.path(), Duration::ZERO).unwrap();
        assert_eq!(removed, 0);
        assert!(dir.path().join("issues.jsonl").exists());
        assert!(dir.path().join("issues.jsonl-wal").exists());
        assert!(dir.path().join("beads.db-wal").exists());
    }

    #[test]
    fn removes_old_jsonl_tmp_files() {
        let dir = TempDir::new().unwrap();
        // beads_rust temp scheme: `issues.jsonl.<pid>.tmp` (decimal digits only)
        let p = dir.path().join("issues.jsonl.12345.tmp");
        std::fs::write(&p, b"orphan").unwrap();
        let removed = sweep_stale_jsonl_temps(dir.path(), Duration::ZERO).unwrap();
        assert_eq!(removed, 1);
        assert!(!p.exists());
    }

    #[test]
    fn keeps_recent_tmp_files() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("issues.jsonl.99999.tmp");
        std::fs::write(&p, b"in-flight").unwrap();
        let removed = sweep_stale_jsonl_temps(dir.path(), Duration::from_secs(3600)).unwrap();
        assert_eq!(removed, 0);
        assert!(p.exists());
    }

    #[test]
    fn ignores_non_pid_lookalikes() {
        let dir = TempDir::new().unwrap();
        // Right shape but non-digit middle — must not be matched.
        let p1 = dir.path().join("issues.jsonl.abc.tmp");
        // Right prefix/suffix but empty middle.
        let p2 = dir.path().join("issues.jsonl..tmp");
        std::fs::write(&p1, b"x").unwrap();
        std::fs::write(&p2, b"x").unwrap();
        let removed = sweep_stale_jsonl_temps(dir.path(), Duration::ZERO).unwrap();
        assert_eq!(removed, 0);
        assert!(p1.exists());
        assert!(p2.exists());
    }

    #[test]
    fn tolerates_missing_dir() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        assert_eq!(
            sweep_stale_jsonl_temps(&nonexistent, Duration::ZERO).unwrap(),
            0
        );
    }
}

/// Cheap filesystem-only probe used to decide whether the boot-time
/// `init_writer_with_flush` sequence has anything to do. The full sequence
/// holds `.write.lock` across `SQLite` open + `sweep_stale_jsonl_temps` +
/// `sync::auto_flush`, then releases it before the WAL checkpoint. It can
/// still take several seconds on a busy beads dir and starve any concurrent
/// process trying to acquire the lock during PM startup. When the filesystem
/// signals no pending work we
/// can skip the lock entirely; the next mutation will trigger a regular
/// `auto_flush` and correct any state we left untouched.
///
/// Returns true (= safe to skip) iff ALL of:
///   * `beads.db` exists (otherwise init must run to create it),
///   * `issues.jsonl` exists (otherwise we owe an initial export),
///   * no `issues.jsonl.<pid>.tmp` files are present (no sweep work pending),
///   * `issues.jsonl` mtime is at least as recent as `beads.db` and any
///     `beads.db-wal` sidecar (no SQLite→JSONL flush is pending).
///
/// Conservative by design: any mtime/stat error or unknown state returns
/// false so the caller takes the slow lock-held path. False-positives only
/// re-introduce the (correct) original behaviour; false-negatives are the
/// only thing that would skip a legitimate flush, and we have no such path.
fn can_skip_init_flush(beads_dir: &Path) -> bool {
    let db_path = beads_dir.join("beads.db");
    if !db_path.exists() {
        return false;
    }
    if pre_open_quick_check(&db_path).is_err() {
        return false;
    }

    let jsonl_path = beads_dir.join("issues.jsonl");
    let Ok(jsonl_meta) = std::fs::metadata(&jsonl_path) else {
        return false;
    };
    let Ok(jsonl_mtime) = jsonl_meta.modified() else {
        return false;
    };

    let Ok(entries) = std::fs::read_dir(beads_dir) else {
        return false;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let name = entry.file_name();
        if let Some(name_str) = name.to_str() {
            if is_jsonl_temp_file(name_str) {
                return false;
            }
        }
    }

    let Ok(conn) =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    let needs_flush = match conn.query_row(
        "SELECT value FROM metadata WHERE key = 'needs_flush'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => value == "true",
        Err(rusqlite::Error::QueryReturnedNoRows) => false,
        Err(_) => return false,
    };
    if needs_flush {
        return false;
    }

    let Ok(db_meta) = std::fs::metadata(&db_path) else {
        return false;
    };
    // Intentionally keep strict `>` checks here (not `>=`) until mtime semantics are revisited.
    if db_meta.modified().ok().is_some_and(|m| m > jsonl_mtime) {
        return false;
    }

    let wal_path = beads_dir.join("beads.db-wal");
    if let Ok(wal_meta) = std::fs::metadata(&wal_path) {
        if wal_meta.modified().ok().is_some_and(|m| m > jsonl_mtime) {
            return false;
        }
    }

    true
}

/// Initialize the writer-side storage state and flush stale `SQLite` changes to
/// JSONL while holding `.write.lock` for the full boot-time sequence.
///
/// Fast path: `can_skip_init_flush` (lock-free, filesystem-only) returns true
/// for a quiet, freshly-exported beads dir. In that common case we never
/// acquire the write lock — startup is O(milliseconds) instead of waiting up
/// to `lock_timeout_ms` for whatever process is doing the actual flush work.
pub(crate) fn init_writer_with_flush(
    beads_dir: &Path,
    lock_timeout_ms: u64,
    stale_tmp_min_age: Duration,
) -> anyhow::Result<()> {
    quarantine_stale_siblings(beads_dir);

    if can_skip_init_flush(beads_dir) {
        tracing::debug!(
            beads_dir = %beads_dir.display(),
            "init_writer_with_flush: fast path — no pending flush, skipping write lock"
        );
        return Ok(());
    }

    let _guard = write_lock::blocking_write_lock_with_timeout(beads_dir, Some(lock_timeout_ms))?;
    let db_path = beads_dir.join("beads.db");
    pre_open_quick_check(&db_path)?;
    let mut storage = SqliteStorage::open_with_timeout(&db_path, Some(lock_timeout_ms))?;
    let _ = sweep_stale_jsonl_temps(beads_dir, stale_tmp_min_age);
    let result = sync::auto_flush(&mut storage, beads_dir);
    drop(storage);
    drop(_guard);
    // Documented exception to the connection actor rule: startup auto-flush
    // runs before BeadsDb exists, so this one-shot checkpoint may still use the
    // legacy helper. Hot-path reads/writes checkpoint on the writer actor.
    wal_checkpoint::checkpoint_wal_truncate_best_effort(&db_path);
    result?;
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_writer_with_flush_runs_clean_on_fresh_dir() {
        let dir = TempDir::new().unwrap();
        init_writer_with_flush(dir.path(), 5_000, Duration::ZERO)
            .expect("fresh dir init should succeed");
    }
}

#[cfg(test)]
mod quarantine_tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn quarantines_stale_db_like_top_level_files_with_manifest() {
        let _guard = env_lock();
        std::env::remove_var("SPUR_BEADS_NO_QUARANTINE");
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("beads.db"), b"live").unwrap();
        fs::write(dir.path().join("issues.jsonl"), b"live").unwrap();
        fs::write(dir.path().join("beads.db.broken-123"), b"broken").unwrap();
        fs::write(dir.path().join("data.db"), b"old").unwrap();
        fs::create_dir(dir.path().join(".br_history")).unwrap();

        quarantine_stale_siblings(dir.path());

        assert!(dir.path().join("beads.db").exists());
        assert!(dir.path().join("issues.jsonl").exists());
        assert!(dir.path().join(".br_history").is_dir());
        assert!(!dir.path().join("beads.db.broken-123").exists());
        assert!(!dir.path().join("data.db").exists());

        let quarantine_root = dir.path().join("quarantine");
        let batch_dir = fs::read_dir(&quarantine_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(batch_dir.join("beads.db.broken-123").exists());
        assert!(batch_dir.join("data.db").exists());

        let manifest: Value =
            serde_json::from_slice(&fs::read(batch_dir.join("quarantine.json")).unwrap()).unwrap();
        let entries = manifest["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| {
            entry["original_path"]
                .as_str()
                .is_some_and(|path| path.ends_with("beads.db.broken-123"))
                && entry["classifier_match"] == "*.broken-*"
        }));
        assert!(entries.iter().any(|entry| {
            entry["original_path"]
                .as_str()
                .is_some_and(|path| path.ends_with("data.db"))
                && entry["classifier_match"] == "data.db"
        }));
    }

    #[test]
    fn quarantine_batch_dir_includes_pid() {
        let _guard = env_lock();
        std::env::remove_var("SPUR_BEADS_NO_QUARANTINE");
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("beads.db.stale-123"), b"stale").unwrap();

        quarantine_stale_siblings(dir.path());

        let quarantine_root = dir.path().join("quarantine");
        let batch_dir = fs::read_dir(&quarantine_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let batch_name = batch_dir.file_name().unwrap().to_str().unwrap();
        assert!(
            batch_name.ends_with(&format!("-{}", std::process::id())),
            "quarantine batch dir should include process id: {batch_name}"
        );
    }

    #[test]
    fn quarantine_can_be_disabled_by_env() {
        let _guard = env_lock();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("beads.db.stale-123"), b"stale").unwrap();
        std::env::set_var("SPUR_BEADS_NO_QUARANTINE", "1");

        quarantine_stale_siblings(dir.path());

        assert!(dir.path().join("beads.db.stale-123").exists());
        assert!(!dir.path().join("quarantine").exists());
        std::env::remove_var("SPUR_BEADS_NO_QUARANTINE");
    }
}

#[cfg(test)]
mod can_skip_init_flush_tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use std::time::SystemTime;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        fs::write(path, b"").unwrap();
    }

    fn set_mtime(path: &Path, t: SystemTime) {
        set_file_mtime(path, FileTime::from_system_time(t)).unwrap();
    }

    fn create_valid_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn returns_false_when_db_missing() {
        let dir = TempDir::new().unwrap();
        // jsonl exists but db does not — must take slow path so init can create db
        touch(&dir.path().join("issues.jsonl"));
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_jsonl_missing() {
        let dir = TempDir::new().unwrap();
        create_valid_db(&dir.path().join("beads.db"));
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_stale_tmp_present() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        create_valid_db(&db);
        touch(&jsonl);
        let now = SystemTime::now();
        set_mtime(&db, now);
        set_mtime(&jsonl, now);
        // A pending sweep target should force the slow path so it gets removed
        touch(&dir.path().join("issues.jsonl.12345.tmp"));
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_plain_export_tmp_present() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        create_valid_db(&db);
        touch(&jsonl);
        let now = SystemTime::now();
        set_mtime(&db, now);
        set_mtime(&jsonl, now);
        assert!(can_skip_init_flush(dir.path()));

        touch(&dir.path().join("issues.jsonl.tmp"));
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_db_newer_than_jsonl() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        touch(&jsonl);
        create_valid_db(&db);
        let old = SystemTime::now() - Duration::from_secs(60);
        let new = SystemTime::now();
        set_mtime(&jsonl, old);
        set_mtime(&db, new);
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_wal_newer_than_jsonl() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let wal = dir.path().join("beads.db-wal");
        let jsonl = dir.path().join("issues.jsonl");
        create_valid_db(&db);
        touch(&jsonl);
        touch(&wal);
        let old = SystemTime::now() - Duration::from_secs(60);
        let new = SystemTime::now();
        set_mtime(&db, old);
        set_mtime(&jsonl, old);
        set_mtime(&wal, new);
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_true_when_quiet_and_jsonl_at_least_as_fresh_as_db() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        create_valid_db(&db);
        touch(&jsonl);
        let old = SystemTime::now() - Duration::from_secs(60);
        let new = SystemTime::now();
        set_mtime(&db, old);
        set_mtime(&jsonl, new);
        assert!(can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_true_when_no_wal_and_jsonl_at_least_as_fresh_as_db() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        create_valid_db(&db);
        touch(&jsonl);
        let t = SystemTime::now() - Duration::from_secs(60);
        set_mtime(&db, t);
        set_mtime(&jsonl, t);
        assert!(can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_needs_flush_metadata_is_true() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO metadata (key, value) VALUES ('needs_flush', 'true')",
                [],
            )
            .unwrap();
        }
        touch(&jsonl);
        let old = SystemTime::now() - Duration::from_secs(60);
        let new = SystemTime::now();
        set_mtime(&db, old);
        set_mtime(&jsonl, new);
        assert!(!can_skip_init_flush(dir.path()));
    }

    #[test]
    fn returns_false_when_db_is_zero_byte() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("beads.db");
        let jsonl = dir.path().join("issues.jsonl");
        touch(&db);
        touch(&jsonl);
        let t = SystemTime::now() - Duration::from_secs(60);
        set_mtime(&db, t);
        set_mtime(&jsonl, t);
        assert!(!can_skip_init_flush(dir.path()));
    }
}
