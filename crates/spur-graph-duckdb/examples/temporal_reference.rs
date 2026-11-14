//! Direct-library, reachable-history reference with bounded temporal shards.
//!
//! Usage: temporal_reference REPO OUTPUT JOBS cli|gix
//! REPO must be an already-frozen, complete checkout (bare is supported), without
//! a prior SPUR commit-index pointer. OUTPUT must not exist; its parent must exist
//! outside REPO. This example never changes the source checkout or refs.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Instant;

use spur_graph::git_walk::{run_full_walk_into, snapshot_refs, GitWalkConfig};
use spur_graph::store::commit_index;
use spur_graph::{
    write_artifact_parquet, GraphArtifactManifest, TemporalShardConfig, TemporalShardSink,
    WalkStrategy, WriteOptions,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let started = Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: temporal_reference REPO OUTPUT JOBS cli|gix".into());
    }
    let repo = Path::new(&args[0]).canonicalize()?;
    let output = Path::new(&args[1]);
    let jobs: NonZeroUsize = args[2].to_str().ok_or("JOBS must be UTF-8")?.parse()?;
    let backend = args[3].to_str().ok_or("BACKEND must be UTF-8")?;
    let use_gix_diff = match backend {
        "cli" => false,
        "gix" => true,
        _ => return Err("BACKEND must be cli or gix".into()),
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent.canonicalize()?.starts_with(&repo) {
        return Err("OUTPUT must be outside the source repository".into());
    }
    if commit_index::load_pointer(&repo)?.is_some() {
        return Err(
            "REPO contains a prior SPUR commit index; supply a fresh isolated checkout".into(),
        );
    }
    let frozen_refs = snapshot_refs(&repo, &["HEAD"])?;
    let head = frozen_refs.get("HEAD").ok_or("HEAD was not resolved")?;
    // A new directory prevents mixing shards or a previous completion marker.
    fs::create_dir(output)?;
    let setup_seconds = started.elapsed().as_secs_f64();
    let mut sink = TemporalShardSink::new(output.to_path_buf(), TemporalShardConfig::default())?;
    let config = GitWalkConfig {
        target_refs: vec!["HEAD".to_owned()],
        walk_strategy: WalkStrategy::Reachable,
        use_gix_diff,
        temporal_jobs: jobs,
        ..GitWalkConfig::default()
    };
    let phase = Instant::now();
    let (graph, commits) = run_full_walk_into(&repo, &config, None, Some(&mut sink))?;
    let walk_seconds = phase.elapsed().as_secs_f64();
    if commits.refs != frozen_refs || snapshot_refs(&repo, &["HEAD"])? != frozen_refs {
        return Err("source HEAD changed during the reference walk".into());
    }
    if graph.commits != commits.commits {
        return Err("graph and commit-index commit records disagree".into());
    }
    if !graph.symbol_snapshots.is_empty() || !graph.temporal_edges.is_empty() {
        return Err("temporal rows were retained instead of streamed to the shard sink".into());
    }
    let phase = Instant::now();
    let shards = sink.finalize()?;
    let shard_count = shards.len();
    let snapshot_count: usize = shards.iter().map(|s| s.row_count_snapshots).sum();
    let edge_count: usize = shards.iter().map(|s| s.row_count_edges).sum();
    let finalize_seconds = phase.elapsed().as_secs_f64();
    let phase = Instant::now();
    write_artifact_parquet(&graph, output, WriteOptions::default(), shards)?;
    let parquet_seconds = phase.elapsed().as_secs_f64();
    let phase = Instant::now();
    // Persist the native commit index beside the artifact, without installing a
    // pointer under the source repository's .spur directory.
    let mut index = BufWriter::new(new_file(&output.join("commit-index.json"))?);
    serde_json::to_writer_pretty(&mut index, &commits)?;
    index.flush()?;
    index.get_ref().sync_all()?;

    let manifest_path = output.join("manifest.json");
    let mut manifest: GraphArtifactManifest =
        serde_json::from_reader(BufReader::new(File::open(&manifest_path)?))?;
    if !manifest.complete
        || manifest.row_counts.commits != commits.commits.len()
        || manifest.row_counts.symbol_snapshots != snapshot_count
        || manifest.row_counts.temporal_edges != edge_count
    {
        return Err("written artifact does not match the completed reference walk".into());
    }
    // The generic writer leaves this optional provenance field unset.
    manifest.indexed_commit_oid = Some(head.clone());
    let mut manifest_file = BufWriter::new(File::create(manifest_path)?);
    serde_json::to_writer_pretty(&mut manifest_file, &manifest)?;
    manifest_file.flush()?;
    manifest_file.get_ref().sync_all()?;
    let metadata_seconds = phase.elapsed().as_secs_f64();
    let summary = serde_json::json!({
        "complete": true,
        "head": head,
        "backend": backend,
        "jobs": jobs.get(),
        "walk_strategy": config.walk_strategy,
        "commits": commits.commits.len(),
        "symbol_snapshots": snapshot_count,
        "temporal_edges": edge_count,
        "diagnostics": graph.diagnostics.len(),
        "temporal_shards": shard_count,
        "phase_seconds": {
            "setup": setup_seconds,
            "walk": walk_seconds,
            "finalize_shards": finalize_seconds,
            "write_parquet": parquet_seconds,
            "write_metadata": metadata_seconds,
            "total": started.elapsed().as_secs_f64(),
        },
    });
    let mut result = BufWriter::new(new_file(&output.join("temporal-reference-result.json"))?);
    serde_json::to_writer_pretty(&mut result, &summary)?;
    result.flush()?;
    result.get_ref().sync_all()?;
    println!("{summary}");
    Ok(())
}

fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
