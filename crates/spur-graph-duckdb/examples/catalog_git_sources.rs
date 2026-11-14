//! Export a frozen Git tree with SPUR's authoritative language classification.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::Command;

use spur_graph::extract::languages::Language;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let repository = args.next().ok_or("expected repository path")?;
    let revision = args.next().ok_or("expected frozen revision")?;
    let destination = args.next().ok_or("expected output JSONL path")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-r", "-l", "-z"])
        .arg(revision)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string().into());
    }
    let mut writer = BufWriter::new(File::create(destination)?);
    let mut entries = 0;
    let mut supported = 0;
    for entry in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let split = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or("missing tree separator")?;
        let metadata: Vec<_> = std::str::from_utf8(&entry[..split])?
            .split_whitespace()
            .collect();
        if metadata.len() != 4 {
            return Err("invalid ls-tree metadata".into());
        }
        let path = std::str::from_utf8(&entry[split + 1..])?;
        let regular_blob = metadata[1] == "blob" && matches!(metadata[0], "100644" | "100755");
        let language = regular_blob
            .then(|| Language::from_path(Path::new(path)))
            .flatten();
        let size = if metadata[3] == "-" {
            None
        } else {
            Some(metadata[3].parse::<u64>()?)
        };
        let row = serde_json::json!({
            "path": path,
            "mode": metadata[0],
            "kind": metadata[1],
            "blob_oid": metadata[2],
            "byte_length": size,
            "language": language.map(|language| format!("{language:?}")),
        });
        writeln!(writer, "{row}")?;
        entries += 1;
        supported += usize::from(language.is_some());
    }
    writer.flush()?;
    println!("catalog entries={entries} supported_source_paths={supported}");
    Ok(())
}
