//! Existing GitCLI symbol-diff policy, with one ordinary reused parse cache.
//!
//! Usage: temporal_symbol_oracle REPO COMMITS_JSONL OUTPUT_JSONL
//! Input rows contain a full hexadecimal commit_oid; additional fields are
//! ignored. Symbol output uses native SymbolSnapshotArtifact and ChangeKind
//! serialization. OUTPUT_JSONL and its .summary.json / .diagnostics.jsonl
//! sidecars must be new files outside the frozen source repository.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use spur_graph::git_walk::{
    check_replace_refs, ensure_not_shallow, file_changes_for_commit, symbol_changes_for_commit,
    SymbolDiffCtx,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let started = Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: temporal_symbol_oracle REPO COMMITS_JSONL OUTPUT_JSONL".into());
    }
    let repo = Path::new(&args[0]).canonicalize()?;
    let destination = Path::new(&args[2]);
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent.canonicalize()?.starts_with(&repo) {
        return Err("OUTPUT_JSONL must be outside the source repository".into());
    }
    ensure_not_shallow(&repo)?;
    check_replace_refs(&repo, false)?;
    let mut requests = BufReader::new(File::open(&args[1])?);
    let summary_path = sidecar(destination, ".summary.json");
    let diagnostic_path = sidecar(destination, ".diagnostics.jsonl");
    if summary_path.exists() || diagnostic_path.exists() {
        return Err("oracle sidecar already exists; choose a fresh OUTPUT_JSONL".into());
    }
    let mut output = BufWriter::new(new_file(destination)?);
    let mut diagnostics = BufWriter::new(new_file(&diagnostic_path)?);
    // Deliberately one context: this is the established GitCLI matching oracle,
    // not an independently reimplemented matcher or a parallel walk.
    let mut context = SymbolDiffCtx::new();
    let setup_seconds = started.elapsed().as_secs_f64();
    let mut line = String::new();
    let mut line_number = 0usize;
    let mut requested_commits = 0usize;
    let mut completed_commits = 0usize;
    let mut file_changes = 0usize;
    let mut symbol_changes = 0usize;
    let mut zero_symbol_commits = 0usize;
    let mut diagnostic_count = 0usize;
    let mut read_seconds = 0.0;
    let mut file_diff_seconds = 0.0;
    let mut symbol_diff_seconds = 0.0;
    let mut write_seconds = 0.0;
    loop {
        let phase = Instant::now();
        line.clear();
        if requests.read_line(&mut line)? == 0 {
            read_seconds += phase.elapsed().as_secs_f64();
            break;
        }
        line_number += 1;
        let row: serde_json::Value = serde_json::from_str(&line)
            .map_err(|error| format!("input line {line_number}: {error}"))?;
        let commit_oid = row["commit_oid"]
            .as_str()
            .ok_or_else(|| format!("input line {line_number}: expected commit_oid string"))?;
        if !matches!(commit_oid.len(), 40 | 64)
            || !commit_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "input line {line_number}: commit_oid must be a full hexadecimal object ID"
            )
            .into());
        }
        requested_commits += 1;
        read_seconds += phase.elapsed().as_secs_f64();
        let phase = Instant::now();
        let files = file_changes_for_commit(&repo, commit_oid)?;
        file_changes += files.len();
        file_diff_seconds += phase.elapsed().as_secs_f64();
        let phase = Instant::now();
        let changes = symbol_changes_for_commit(&repo, commit_oid, &files, &mut context)?;
        symbol_diff_seconds += phase.elapsed().as_secs_f64();
        zero_symbol_commits += usize::from(changes.is_empty());
        symbol_changes += changes.len();
        let phase = Instant::now();
        for change in changes {
            let row = serde_json::json!({
                "commit_oid": commit_oid,
                "parent_sha": change.parent_sha,
                "snapshot": change.snapshot,
                "change_kind": change.change_kind,
            });
            serde_json::to_writer(&mut output, &row)?;
            output.write_all(b"\n")?;
        }
        for diagnostic in &context.diagnostics()[diagnostic_count..] {
            let row = serde_json::json!({"commit_oid": commit_oid, "diagnostic": diagnostic});
            serde_json::to_writer(&mut diagnostics, &row)?;
            diagnostics.write_all(b"\n")?;
        }
        diagnostic_count = context.diagnostics().len();
        write_seconds += phase.elapsed().as_secs_f64();
        completed_commits += 1;
    }
    let phase = Instant::now();
    output.flush()?;
    diagnostics.flush()?;
    output.get_ref().sync_all()?;
    diagnostics.get_ref().sync_all()?;
    write_seconds += phase.elapsed().as_secs_f64();
    let summary = serde_json::json!({
        "complete": true,
        "backend": "cli",
        "requested_commits": requested_commits,
        "completed_commits": completed_commits,
        "file_changes": file_changes,
        "symbol_changes": symbol_changes,
        "zero_symbol_commits": zero_symbol_commits,
        "diagnostics": diagnostic_count,
        "phase_seconds": {
            "setup": setup_seconds,
            "read_requests": read_seconds,
            "file_diff": file_diff_seconds,
            "symbol_diff": symbol_diff_seconds,
            "write_rows": write_seconds,
            "total": started.elapsed().as_secs_f64(),
        },
    });
    let mut summary_file = BufWriter::new(new_file(&summary_path)?);
    serde_json::to_writer_pretty(&mut summary_file, &summary)?;
    summary_file.flush()?;
    summary_file.get_ref().sync_all()?;
    println!("{summary}");
    Ok(())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
