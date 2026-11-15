//! Independent one-file native oracle. JSONL is test transport, never SQL output.
//!
//! Usage: reference_file_facts INPUT.jsonl OUTPUT.jsonl
//! Each input is {"path":"relative/file.rs","source":"UTF-8 source"}; each
//! output is the five native GraphFacts collections, in input/native list order.
//! Every record gets a fresh, non-Git temporary root, removed before the next.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use spur_graph::build_facts_for_paths;

struct IsolatedRoot(PathBuf);

impl IsolatedRoot {
    fn new() -> std::io::Result<Self> {
        loop {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let path = std::env::temp_dir().join(format!(
                "spur-file-facts-oracle-{}-{}",
                std::process::id(),
                nonce.as_nanos()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for IsolatedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn relative_path(path: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Keep supplied fixtures inside the fresh root on every supported host.
    if path.contains(['\\', ':', '\0']) {
        return Err("oracle fixtures require a relative logical path".into());
    }
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return Err("oracle fixture path escapes its isolated root".into()),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err("oracle fixture path is empty".into());
    }
    Ok(relative)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or("expected input JSONL path")?;
    let output = args.next().ok_or("expected output JSONL path")?;
    if args.next().is_some() {
        return Err("expected exactly input and output JSONL paths".into());
    }
    let input = BufReader::new(File::open(input)?);
    let mut output = BufWriter::new(File::create(output)?);
    for line in input.lines() {
        let fixture: Value = serde_json::from_str(&line?)?;
        let path = relative_path(fixture["path"].as_str().ok_or("expected path string")?)?;
        let source = fixture["source"].as_str().ok_or("expected source string")?;
        let root = IsolatedRoot::new()?;
        let destination = root.0.join(&path);
        fs::create_dir_all(destination.parent().ok_or("expected file parent")?)?;
        fs::write(destination, source.as_bytes())?;
        // Deliberately independent of BytesExtractor::extract_file_facts.
        // One selected file, fresh builder and native pending-edge resolution.
        let facts = build_facts_for_paths(&root.0, &[path])?;
        let row = json!({
            "nodes": facts.nodes,
            "edges": facts.edges,
            "spans": facts.spans,
            "macro_invocations": facts.macro_invocations,
            "macro_scopes": facts.macro_scopes,
        });
        writeln!(output, "{row}")?;
    }
    output.flush()?;
    Ok(())
}
