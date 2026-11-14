//! Independent reference for a Git `cat-file --batch` frame.
//!
//! This validation tool calls SPUR directly, without loading DuckDB. Its JSONL
//! output is only a test oracle for comparing every typed extension field.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::BytesExtractor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let frame = args.next().ok_or("expected input frame path")?;
    let destination = args.next().ok_or("expected output JSONL path")?;
    let mut input = BufReader::new(File::open(frame)?);
    let mut output = BufWriter::new(File::create(destination)?);
    let mut parser = BytesExtractor::for_language(Language::Cpp)?;
    let mut blobs = 0;
    let mut symbols = 0;
    loop {
        let mut header = String::new();
        if input.read_line(&mut header)? == 0 {
            break;
        }
        let parts: Vec<_> = header.split_whitespace().collect();
        if parts.len() != 3 || parts[1] != "blob" {
            return Err(format!("invalid object header: {header:?}").into());
        }
        let length = parts[2].parse::<usize>()?;
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        let mut separator = [0];
        input.read_exact(&mut separator)?;
        if separator != [b'\n'] {
            return Err("invalid object separator".into());
        }
        for symbol in parser.extract(Path::new("source.cpp"), &bytes)? {
            let row = serde_json::json!({
                "blob_oid": parts[0],
                "entity_name": symbol.entity_name,
                "symbol_kind": symbol.symbol_kind,
                "enclosing_scope": symbol.enclosing_scope,
                "byte_start": symbol.byte_range[0],
                "byte_end": symbol.byte_range[1],
                "line_start": symbol.line_range[0],
                "line_end": symbol.line_range[1],
                "anchor_hash": symbol.anchor_hash,
                "tokens": symbol.tokens,
            });
            writeln!(output, "{row}")?;
            symbols += 1;
        }
        blobs += 1;
    }
    output.flush()?;
    println!("reference blobs={blobs} symbols={symbols}");
    Ok(())
}
