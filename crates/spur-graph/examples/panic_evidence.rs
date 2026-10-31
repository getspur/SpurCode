//! Opt-in JSON sidecar runner. Reads one request from stdin, writes one response
//! to stdout. Snapshot constructs a fixture artifact in memory, never on disk.
use std::io::{Read, Write};
use std::path::{Component, PathBuf};

use anyhow::{ensure, Context as _};
use serde::Deserialize;
use serde_json::{json, Value};
use spur_graph::panic_evidence::*;
use spur_graph::GraphIndexArtifact;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    file_path: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    path: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Snapshot {
        root: PathBuf,
        source_paths: Vec<String>,
    },
    Collect {
        graph: GraphIndexArtifact,
        sources: Vec<Source>,
    },
    Identity {
        identity: CaptureBuildIdentity,
    },
    Import {
        graph: GraphIndexArtifact,
        sources: Vec<Source>,
        method_sites: MethodCallSiteEvidenceSet,
        inputs: Vec<Input>,
        identity: CaptureBuildIdentity,
        expected_build_id: String,
        raw_export: String,
    },
    Validate {
        graph: GraphIndexArtifact,
        sources: Vec<Source>,
        method_sites: MethodCallSiteEvidenceSet,
        inputs: Vec<Input>,
        identity: CaptureBuildIdentity,
        expected_build_id: String,
        raw_export: String,
        compiler_evidence: CompilerEvidenceSet,
    },
}

fn source_refs(sources: &[Source]) -> Vec<PanicEvidenceSource<'_>> {
    sources
        .iter()
        .map(|s| PanicEvidenceSource {
            file_path: &s.file_path,
            bytes: s.text.as_bytes(),
        })
        .collect()
}

/// Also used by integration tests so the JSON boundary has the same validator
/// as the actual stdin/stdout command.
pub fn execute(request: Value) -> anyhow::Result<Value> {
    match serde_json::from_value::<Request>(request).context("invalid panic evidence request")? {
        Request::Snapshot { root, source_paths } => {
            let root = root.canonicalize()?;
            let mut sources = Vec::new();
            for file_path in source_paths {
                let path = std::path::Path::new(&file_path);
                ensure!(
                    path.is_relative()
                        && path.components().all(|c| matches!(c, Component::Normal(_))),
                    "noncanonical source path"
                );
                let physical = root.join(path).canonicalize()?;
                ensure!(physical.starts_with(&root), "source escapes fixture root");
                sources.push(Source {
                    file_path,
                    text: std::fs::read_to_string(physical)?,
                });
            }
            let (facts, _) = spur_graph::build_facts(&root, None)?;
            let graph = spur_graph::artifact_from_facts(&facts, &root)?;
            let method_sites = collect_method_call_sites(&graph, &source_refs(&sources))?;
            Ok(json!({"graph":graph,"method_sites":method_sites}))
        }
        Request::Collect { graph, sources } => {
            Ok(json!({"method_sites":collect_method_call_sites(&graph,&source_refs(&sources))?}))
        }
        Request::Identity { identity } => Ok(json!({"build_id":identity.build_id()?})),
        Request::Import {
            graph,
            sources,
            method_sites,
            inputs,
            identity,
            expected_build_id,
            raw_export,
        } => {
            let sources = source_refs(&sources);
            let inputs: Vec<_> = inputs
                .iter()
                .map(|i| CaptureInputBytes {
                    path: &i.path,
                    bytes: i.text.as_bytes(),
                })
                .collect();
            let compiler_evidence = import_cga(CgaImportRequest {
                identity: &identity,
                expected_build_id: &expected_build_id,
                raw_export: raw_export.as_bytes(),
                inputs: &inputs,
                graph: &graph,
                sources: &sources,
                method_sites: &method_sites,
            })?;
            Ok(json!({"compiler_evidence":compiler_evidence}))
        }
        Request::Validate {
            graph,
            sources,
            method_sites,
            inputs,
            identity,
            expected_build_id,
            raw_export,
            compiler_evidence,
        } => {
            let sources = source_refs(&sources);
            let inputs: Vec<_> = inputs
                .iter()
                .map(|i| CaptureInputBytes {
                    path: &i.path,
                    bytes: i.text.as_bytes(),
                })
                .collect();
            compiler_evidence.validate(CgaImportRequest {
                identity: &identity,
                expected_build_id: &expected_build_id,
                raw_export: raw_export.as_bytes(),
                inputs: &inputs,
                graph: &graph,
                sources: &sources,
                method_sites: &method_sites,
            })?;
            Ok(json!({"valid":true,"build_id":expected_build_id}))
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let response = execute(serde_json::from_str(&input)?)?;
    serde_json::to_writer(std::io::stdout().lock(), &response)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}
