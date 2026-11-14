//! Inventory an entire frozen reachable history using one bulk Git diff process.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use spur_graph::extract::languages::Language;

fn field(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut value = Vec::new();
    if reader.read_until(0, &mut value)? == 0 {
        return Ok(None);
    }
    if value.pop() != Some(0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "unterminated Git field",
        ));
    }
    Ok(Some(value))
}

fn write_row(writer: &mut impl Write, value: serde_json::Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, &value)?;
    writer.write_all(b"\n")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("expected: repository frozen-revision output-directory".into());
    }
    let repository = Path::new(&args[0]);
    let destination = Path::new(&args[2]);
    let revision = args[1].to_str().ok_or("revision must be UTF-8")?;
    spur_graph::git_walk::ensure_not_shallow(repository)?;
    spur_graph::git_walk::check_replace_refs(repository, false)?;
    if destination.exists() && fs::read_dir(destination)?.next().is_some() {
        return Err("inventory destination must be empty; preserve existing generations".into());
    }
    fs::create_dir_all(destination)?;
    let started = Instant::now();
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args([
            "log",
            "--topo-order",
            "--reverse",
            "--format=%H%x00%T%x00%P%x00%at%x00%s",
        ])
        .arg(&args[1])
        .arg("--")
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string().into());
    }
    let mut commits = BufWriter::new(File::create(destination.join("commits.jsonl"))?);
    let mut parent_rows = BufWriter::new(File::create(destination.join("comparisons.jsonl"))?);
    let mut comparisons = Vec::new();
    let mut input = Vec::new();
    let mut commit_count = 0;
    for line in output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let parts: Vec<_> = line.splitn(5, |byte| *byte == 0).collect();
        if parts.len() != 5 {
            return Err("invalid Git commit record".into());
        }
        let oid = std::str::from_utf8(parts[0])?.to_owned();
        let parents: Vec<_> = std::str::from_utf8(parts[2])?
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        write_row(
            &mut commits,
            json!({"commit_oid":oid,"ordinal":commit_count,"tree_oid":std::str::from_utf8(parts[1])?,"parents":parents,"author_time":std::str::from_utf8(parts[3])?.parse::<i64>()?,"summary":String::from_utf8_lossy(parts[4])}),
        )?;
        for (index, parent) in if parents.is_empty() {
            vec![(0, None)]
        } else {
            parents
                .iter()
                .enumerate()
                .map(|(i, p)| (i, Some(p.clone())))
                .collect()
        } {
            let comparison = comparisons.len();
            write_row(
                &mut parent_rows,
                json!({"comparison_id":comparison,"commit_oid":oid,"parent_index":index,"parent_oid":parent}),
            )?;
            input.extend_from_slice(oid.as_bytes());
            if let Some(parent) = &parent {
                input.push(b' ');
                input.extend_from_slice(parent.as_bytes());
            }
            input.push(b'\n');
            comparisons.push((oid.clone(), parent, index));
        }
        commit_count += 1;
    }
    commits.flush()?;
    parent_rows.flush()?;
    let commit_seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "commits={commit_count} comparisons={} commit_seconds={commit_seconds:.3}",
        comparisons.len()
    );

    let mut child = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args([
            "diff-tree",
            "--stdin",
            "--root",
            "-r",
            "--raw",
            "-z",
            "--no-abbrev",
            "--find-renames",
            "--always",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("missing Git stdin")?;
    let sender = std::thread::spawn(move || stdin.write_all(&input));
    let mut reader = BufReader::new(child.stdout.take().ok_or("missing Git stdout")?);
    let mut changes = BufWriter::new(File::create(destination.join("changes.jsonl"))?);
    let mut current = None;
    let mut comparison_count = 0;
    let mut change_count = 0_u64;
    let mut last_report = Instant::now();
    while let Some(header) = field(&mut reader)? {
        if header.first() != Some(&b':') {
            let expected = comparisons
                .get(comparison_count)
                .ok_or("extra Git comparison header")?;
            if header != expected.0.as_bytes() {
                return Err("Git comparison order mismatch".into());
            }
            current = Some(comparison_count);
            comparison_count += 1;
            if last_report.elapsed() >= Duration::from_secs(1) {
                eprintln!(
                    "comparisons={comparison_count}/{} changes={change_count} seconds={:.3}",
                    comparisons.len(),
                    started.elapsed().as_secs_f64()
                );
                last_report = Instant::now();
            }
            continue;
        }
        let comparison_id = current.ok_or("change before comparison header")?;
        let (oid, parent, parent_index) = &comparisons[comparison_id];
        let fields: Vec<_> = std::str::from_utf8(&header)?.split_whitespace().collect();
        if fields.len() != 5 {
            return Err("invalid Git raw change".into());
        }
        let first_path = String::from_utf8(field(&mut reader)?.ok_or("missing path")?)?;
        let renamed = fields[4].starts_with(['R', 'C']);
        let path = if renamed {
            String::from_utf8(field(&mut reader)?.ok_or("missing rename destination")?)?
        } else {
            first_path.clone()
        };
        let gitlink = fields[0] == ":160000" || fields[1] == "160000";
        let language = if gitlink {
            None
        } else {
            Language::from_path(Path::new(&path)).map(|value| format!("{value:?}"))
        };
        let old_oid = (!fields[2].bytes().all(|byte| byte == b'0')).then_some(fields[2]);
        let new_oid = (!fields[3].bytes().all(|byte| byte == b'0')).then_some(fields[3]);
        let old_path = if old_oid.is_none() {
            None
        } else if renamed {
            Some(first_path.as_str())
        } else {
            Some(path.as_str())
        };
        write_row(
            &mut changes,
            json!({"change_id":change_count,"comparison_id":comparison_id,"commit_oid":oid,"parent_oid":parent,"parent_index":parent_index,"status":fields[4],"old_mode":fields[0].trim_start_matches(':'),"new_mode":fields[1],"old_blob_oid":old_oid,"new_blob_oid":new_oid,"old_path":old_path,"path":path,"language":language,"gitlink":gitlink}),
        )?;
        change_count += 1;
    }
    sender.join().map_err(|_| "Git input writer panicked")??;
    if !child.wait()?.success() {
        return Err("Git diff process failed".into());
    }
    if comparison_count != comparisons.len() {
        return Err("missing Git comparison headers".into());
    }
    changes.flush()?;
    let summary = json!({"state":"complete","head":revision,"commits":commit_count,"comparisons":comparison_count,"changes":change_count,"commit_seconds":commit_seconds,"wall_seconds":started.elapsed().as_secs_f64(),"policy":"reachable; every parent; roots and empty comparisons; Git CLI default find-renames; UTF-8 paths; SPUR language registry"});
    fs::write(
        destination.join("inventory-summary.json"),
        format!("{summary}\n"),
    )?;
    println!("{summary}");
    Ok(())
}
