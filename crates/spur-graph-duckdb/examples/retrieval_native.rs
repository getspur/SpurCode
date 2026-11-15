//! Persistent reference for the retrieval parity experiment (no SQL/candidate data).
//!
//! Build from the repository root, independently of the remote check/test:
//! `SPUR_REMOTE=0 scripts/spur-cargo build --manifest-path crates/spur-graph-duckdb/Cargo.toml --example retrieval_native`
//! `retrieval_native prepare tests/fixtures/retrieval/cases.json /private/tmp/fresh-corpus`
//! `retrieval_native serve /private/tmp/fresh-corpus < requests.jsonl > native.jsonl`
//!
//! Shared spec: {files:[{path,source}],requests:[{case_id,tool,args}]}.
//! Source strings are written as UTF-8 bytes without newline conversion. The output
//! directory must not exist. It becomes a standalone Git repository, never a linked
//! worktree or registered project. `.retrieval/spec.json` is the shared input;
//! `.retrieval/native/` holds reference-only provenance and raw warmup responses.
//!
//! Serve verifies the frozen sources, HEAD, and artifact, then warms the complete
//! request matrix in one production module before reading stdin. Each JSONL reply
//! echoes the request and adds {body,error,elapsed_ns}; exactly one of body/error is
//! non-null. Errors preserve all fields from production `into_error_response`.
//! elapsed_ns includes dispatch and production error conversion, excludes JSON
//! parsing/writing, verification, construction, and startup warmup. Raw response
//! fields are never normalized. Malformed lines produce a separate protocol_error.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Component, Path};
use std::process::Command;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spur_graph::mcp::{with_worktree_root_for_request, GraphMcpModule};
use spur_graph::{
    artifact_from_facts, build_facts, git_blob_oid, read_artifact_parquet, read_current_pointer,
    store::cache::publish_worktree_rebuild, write_artifact_parquet, GitCtx, WriteOptions,
};

const SPEC: &str = ".retrieval/spec.json";
const NATIVE: &str = ".retrieval/native";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    files: Vec<Source>,
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source {
    path: String,
    source: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    case_id: String,
    tool: String,
    args: Value,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut out = BufWriter::new(fs::File::create(path)?);
    serde_json::to_writer_pretty(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

// Ignore inherited Git routing and user configuration, including hooks, filters,
// signing, autocrlf, and alternate object directories. This applies to preparation
// and verification subprocesses; serve clears GIT_* before production dispatch.
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
    {
        cmd.env_remove(key);
    }
    let output = cmd
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Retrieval fixture")
        .env("GIT_AUTHOR_EMAIL", "retrieval@example.invalid")
        .env("GIT_COMMITTER_NAME", "Retrieval fixture")
        .env("GIT_COMMITTER_EMAIL", "retrieval@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-11-15T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-15T00:00:00Z")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git(root, args)?)?.trim().to_owned())
}

fn validate(spec: &Spec) -> Result<()> {
    ensure!(!spec.files.is_empty(), "spec.files must not be empty");
    let mut paths = BTreeSet::new();
    for file in &spec.files {
        let path = Path::new(&file.path);
        ensure!(
            !file.path.is_empty() && !file.path.contains(['\\', '\0']),
            "invalid path"
        );
        ensure!(
            path.components().all(|c| matches!(c, Component::Normal(_))),
            "unsafe path: {}",
            file.path
        );
        ensure!(path.components().all(|c| !matches!(c, Component::Normal(s) if [".git", ".spur", ".retrieval"].iter().any(|reserved| s.as_encoded_bytes().eq_ignore_ascii_case(reserved.as_bytes())))), "reserved path: {}", file.path);
        ensure!(paths.insert(&file.path), "duplicate path: {}", file.path);
    }
    let mut ids = BTreeSet::new();
    for request in &spec.requests {
        ensure!(
            !request.case_id.is_empty() && ids.insert(&request.case_id),
            "empty/duplicate case_id"
        );
        ensure!(request.args.is_object(), "request args must be an object");
    }
    Ok(())
}

fn source_manifest(root: &Path, spec: &Spec) -> Result<Vec<Value>> {
    spec.files.iter().map(|file| {
        let bytes = fs::read(root.join(&file.path))?;
        ensure!(bytes == file.source.as_bytes(), "source changed: {}", file.path);
        let committed = git(root, &["show", &format!("HEAD:{}", file.path)])?;
        ensure!(bytes == committed, "committed source differs: {}", file.path);
        Ok(json!({"path":file.path,"bytes":bytes.len(),"sha256":sha256(&bytes),"git_blob_oid":git_blob_oid(&bytes)}))
    }).collect()
}

fn directory_hashes(root: &Path, dir: &Path) -> Result<Vec<Value>> {
    let mut entries = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    let mut rows = Vec::new();
    for path in entries {
        let kind = fs::symlink_metadata(&path)?.file_type();
        ensure!(!kind.is_symlink(), "artifact symlink: {}", path.display());
        if kind.is_dir() {
            rows.extend(directory_hashes(root, &path)?);
        } else {
            ensure!(kind.is_file(), "non-file artifact");
            let bytes = fs::read(&path)?;
            rows.push(json!({"path":path.strip_prefix(root)?,"bytes":bytes.len(),"sha256":sha256(&bytes)}));
        }
    }
    Ok(rows)
}

fn verify(root: &Path) -> Result<(Spec, Value)> {
    ensure!(
        root.join(".git").is_dir() && !root.join(".git/commondir").exists(),
        "expected standalone Git repository"
    );
    ensure!(
        Path::new(&git_text(root, &["rev-parse", "--show-toplevel"])?) == root,
        "wrong repository root"
    );
    ensure!(
        Path::new(&git_text(root, &["rev-parse", "--git-common-dir"])?) == Path::new(".git"),
        "shared Git directory"
    );
    ensure!(
        git(root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty(),
        "corpus is dirty"
    );
    let spec_bytes = fs::read(root.join(SPEC))?;
    let spec: Spec = serde_json::from_slice(&spec_bytes)?;
    validate(&spec)?;
    let provenance: Value =
        serde_json::from_slice(&fs::read(root.join(NATIVE).join("provenance.json"))?)?;
    ensure!(
        provenance["spec_sha256"] == sha256(&spec_bytes),
        "spec changed"
    );
    ensure!(
        provenance["corpus_root"] == root.to_string_lossy().as_ref(),
        "corpus moved"
    );
    ensure!(
        provenance["head_oid"] == git_text(root, &["rev-parse", "HEAD"])?,
        "HEAD changed"
    );
    ensure!(
        provenance["sources"] == json!(source_manifest(root, &spec)?),
        "source manifest changed"
    );
    let tracked = git(root, &["ls-files", "-z"])?;
    let tracked: BTreeSet<_> = tracked
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect();
    let expected: BTreeSet<_> = spec.files.iter().map(|f| f.path.as_bytes()).collect();
    ensure!(
        tracked == expected,
        "tracked corpus differs from complete shared file set"
    );
    let artifact_dir = read_current_pointer(root)?;
    ensure!(
        artifact_dir.starts_with(root.join(".spur/graph")),
        "artifact escaped isolated corpus"
    );
    ensure!(
        provenance["artifact_files"] == json!(directory_hashes(root, &artifact_dir)?),
        "artifact bytes changed"
    );
    let pointer: Value =
        serde_json::from_slice(&fs::read(root.join(".spur/graph-index.pointer.json"))?)?;
    ensure!(
        provenance["pointer"] == pointer && pointer["indexed_commit_oid"] == provenance["head_oid"],
        "artifact HEAD mismatch"
    );
    Ok((spec, provenance))
}

async fn dispatch(module: &GraphMcpModule, root: &Path, request: &Request) -> Value {
    with_worktree_root_for_request(root.to_path_buf(), async {
        let started = Instant::now();
        let (body, error) = match module.dispatch(&request.tool, request.args.clone()).await {
            Ok(body) => (body, Value::Null),
            Err(error) => {
                let error = error.into_error_response().await;
                (
                    Value::Null,
                    json!({"code":error.code,"message":error.message,"data":error.data}),
                )
            }
        };
        let elapsed_ns = started.elapsed().as_nanos();
        json!({"case_id":request.case_id,"tool":request.tool,"args":request.args,
            "body":body,"error":error,"elapsed_ns":elapsed_ns})
    })
    .await
}

async fn warm(
    module: &GraphMcpModule,
    root: &Path,
    spec: &Spec,
    mut out: impl Write,
) -> Result<()> {
    for request in &spec.requests {
        serde_json::to_writer(&mut out, &dispatch(module, root, request).await)?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}

async fn prepare(spec_path: &Path, destination: &Path) -> Result<Value> {
    let input = fs::read(spec_path)?;
    let spec: Spec = serde_json::from_slice(&input)?;
    validate(&spec)?;
    // create_dir refuses existing directories, including existing repositories.
    fs::create_dir(destination).context("destination must be a fresh, nonexistent directory")?;
    let root = destination.canonicalize()?;
    git(
        &root,
        &[
            "init",
            "--quiet",
            "--template=",
            "--initial-branch=frozen",
            "--object-format=sha1",
        ],
    )?;
    git(&root, &["config", "core.autocrlf", "false"])?;
    git(&root, &["config", "core.hooksPath", "/dev/null"])?;
    fs::create_dir_all(root.join(".git/info"))?;
    fs::write(root.join(".git/info/exclude"), "/.spur/\n/.retrieval/\n")?;
    for file in &spec.files {
        let path = root.join(&file.path);
        fs::create_dir_all(path.parent().context("source parent")?)?;
        fs::write(path, file.source.as_bytes())?;
        // --force permits input files intentionally listed by a supplied .gitignore.
        git(&root, &["add", "--force", "--", &file.path])?;
    }
    git(
        &root,
        &[
            "commit",
            "--quiet",
            "-m",
            "Freeze complete retrieval corpus",
        ],
    )?;
    fs::create_dir_all(root.join(NATIVE))?;
    fs::write(root.join(SPEC), &input)?;
    let head = git_text(&root, &["rev-parse", "HEAD"])?;
    let sources = source_manifest(&root, &spec)?;
    let started = Instant::now();
    let (facts, _) = build_facts(&root, None)?;
    let artifact = artifact_from_facts(&facts, &root)?;
    let extraction_ns = started.elapsed().as_nanos();
    for entry in &artifact.file_manifests {
        let file = spec
            .files
            .iter()
            .find(|f| f.path == entry.path)
            .context("indexed file outside spec")?;
        ensure!(
            entry.content_oid == git_blob_oid(file.source.as_bytes()),
            "indexed source hash mismatch"
        );
    }
    let started = Instant::now();
    let artifact_dir = write_artifact_parquet(
        &artifact,
        &root.join(".spur/graph").join(&artifact.graph_content_hash),
        WriteOptions::default(),
        Vec::new(),
    )?;
    publish_worktree_rebuild(
        &artifact,
        &root,
        Some(&GitCtx {
            worktree_root: root.clone(),
            git_common_dir: root.join(".git"),
            head_oid: head.clone(),
        }),
        &artifact_dir,
    )?;
    let persistence_ns = started.elapsed().as_nanos();
    let stored = read_artifact_parquet(&artifact_dir)?;
    ensure!(
        stored.graph_content_hash == artifact.graph_content_hash,
        "artifact readback mismatch"
    );
    let pointer: Value =
        serde_json::from_slice(&fs::read(root.join(".spur/graph-index.pointer.json"))?)?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let provenance = json!({
        "version":1,"corpus_root":root,"head_oid":head,"spec_sha256":sha256(&input),
        "sources":sources,"pointer":pointer,"graph_content_hash":artifact.graph_content_hash,
        "indexed_files":artifact.file_manifests,"symbol_count":artifact.symbols.len(),"edge_count":artifact.edges.len(),
        "artifact_files":directory_hashes(&root, &artifact_dir)?,
        "construction":{"extraction_ns":extraction_ns,"persistence_ns":persistence_ns},
        "implementation":{"executable":executable,"executable_sha256":sha256(&fs::read(&executable)?),
            "example_source_sha256":sha256(include_bytes!("retrieval_native.rs")),
            "cargo_toml_sha256":sha256(include_bytes!("../Cargo.toml")),
            "cargo_lock_sha256":sha256(include_bytes!("../Cargo.lock")),
            "arch":std::env::consts::ARCH,"os":std::env::consts::OS,
            "checkout_head_at_prepare":git_text(Path::new(env!("CARGO_MANIFEST_DIR")), &["rev-parse", "HEAD"]).ok()},
        "oracle":"GraphMcpModule::default().dispatch + with_worktree_root_for_request",
        "timing":"dispatch plus production error conversion; JSON parse/write and startup warmup excluded",
        "candidate_inputs":[SPEC],"native_only":NATIVE
    });
    write_json(&root.join(NATIVE).join("provenance.json"), &provenance)?;
    let module = GraphMcpModule::default();
    warm(
        &module,
        &root,
        &spec,
        BufWriter::new(fs::File::create(root.join(NATIVE).join("warmup.jsonl"))?),
    )
    .await?;
    verify(&root)?;
    Ok(provenance)
}

async fn serve(root: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let (spec, provenance) = verify(&root)?;
    let module = GraphMcpModule::default();
    let started = Instant::now();
    warm(&module, &root, &spec, io::sink()).await?;
    eprintln!(
        "{}",
        json!({"ready":true,"head_oid":provenance["head_oid"],"spec_sha256":provenance["spec_sha256"],"startup_warmup_ns":started.elapsed().as_nanos()})
    );
    let mut out = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => dispatch(&module, &root, &request).await,
            Err(error) => json!({"protocol_error":error.to_string()}),
        };
        serde_json::to_writer(&mut out, &response)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    // Detect accidental writes or source/artifact mutation over the session.
    verify(&root)?;
    Ok(())
}

fn main() -> Result<()> {
    // One-time process startup, before any production Git work. No parent state
    // is changed; these are the child oracle's environment and working directory.
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
    {
        std::env::remove_var(key);
    }
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    tokio::runtime::Runtime::new()?.block_on(run())
}

async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [command, spec, destination] if command == "prepare" => {
            let provenance = prepare(Path::new(spec), Path::new(destination)).await?;
            serde_json::to_writer(io::stdout().lock(), &provenance)?;
            println!();
        }
        [command, root] if command == "serve" => serve(Path::new(root)).await?,
        _ => bail!(
            "usage: retrieval_native prepare <spec.json> <fresh-output-dir> | serve <corpus-dir>"
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Corpus(PathBuf);
    impl Corpus {
        fn fresh() -> Self {
            Self(std::env::temp_dir().join(format!(
                "retrieval-native-test-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            )))
        }
    }
    impl Drop for Corpus {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn refuses_escape_reserved_and_duplicate_inputs() {
        for path in [
            "../outside.rs",
            "/outside.rs",
            ".git/config",
            ".GIT/config",
            ".spur/graph/x",
            ".retrieval/spec.json",
        ] {
            let spec = Spec {
                files: vec![Source {
                    path: path.into(),
                    source: "".into(),
                }],
                requests: vec![],
            };
            assert!(validate(&spec).is_err(), "accepted {path}");
        }
        let spec: Spec = serde_json::from_value(json!({"files":[{"path":"a.rs","source":""},{"path":"a.rs","source":""}],"requests":[]})).unwrap();
        assert!(validate(&spec).is_err());
    }

    #[tokio::test]
    async fn actual_handlers_frozen_fixture_positive_negative_and_integrity_smoke() -> Result<()> {
        let corpus = Corpus::fresh();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/retrieval/cases.json");
        let provenance = prepare(&fixture, &corpus.0).await?;
        let root = corpus.0.canonicalize()?;
        let (spec, _) = verify(&root)?;
        let warmup = fs::read_to_string(root.join(NATIVE).join("warmup.jsonl"))?;
        let rows: BTreeMap<String, Value> = warmup
            .lines()
            .map(|line| {
                let row: Value = serde_json::from_str(line).unwrap();
                (row["case_id"].as_str().unwrap().to_owned(), row)
            })
            .collect();
        assert_eq!(rows.len(), spec.requests.len());
        let body = |id: &str| {
            assert!(rows[id]["error"].is_null(), "{id}: {}", rows[id]);
            rows[id]["body"].clone()
        };
        assert_eq!(body("search_exact")["total_matches"], 1);
        assert!(
            body("search_prefix")["candidates"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
        assert_eq!(body("search_empty")["total_matches"], 0);
        assert_eq!(body("search_wrong_file")["total_matches"], 0);
        assert_eq!(body("search_wrong_kind")["total_matches"], 0);
        assert_eq!(body("search_method")["total_matches"], 1);
        assert_eq!(
            body("search_file")["candidates"].as_array().unwrap().len(),
            1
        );
        let limited = body("search_limit");
        assert_eq!(limited["candidates"].as_array().unwrap().len(), 1);
        assert!(limited["total_matches"].as_u64().unwrap() > 1);
        assert_eq!(
            body("read_exact")["source"],
            "pub fn same_file_entry() -> u32 {\n    same_file_leaf() + same_file_leaf()\n}\n"
        );
        let crlf = body("read_crlf");
        assert_eq!(
            crlf["source"],
            "pub fn crlf_leaf() -> &'static str {\r\n    \"naïve café 🦀\"\r\n}\r\n"
        );
        assert!(body("read_unicode")["source"]
            .as_str()
            .unwrap()
            .contains("Xin chào 🦀"));
        assert!(body("read_method")["source"]
            .as_str()
            .unwrap()
            .contains("pub fn run"));
        assert!(!body("callees_method")["callees"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(body("read_macro")["source"]
            .as_str()
            .unwrap()
            .contains("macro_rules! local_emit"));
        // Check every full/source-format read against exact frozen byte slices,
        // independently of the source helper in the production implementation.
        for request in &spec.requests {
            if request.tool != "code_read_symbol" {
                continue;
            }
            let row = &rows[&request.case_id];
            let b = &row["body"];
            if !row["error"].is_null() || b.get("source").is_none() {
                continue;
            }
            let file = spec
                .files
                .iter()
                .find(|f| Some(f.path.as_str()) == request.args["path"].as_str())
                .unwrap();
            let range = b.get("line_range").unwrap_or(&b["range"]);
            let (start, end) = if range.is_object() {
                (
                    range["start"].as_u64().unwrap(),
                    range["end"].as_u64().unwrap(),
                )
            } else {
                (range[0].as_u64().unwrap(), range[1].as_u64().unwrap())
            };
            let mut offsets = vec![0];
            offsets.extend(
                file.source
                    .bytes()
                    .enumerate()
                    .filter(|(_, b)| *b == b'\n')
                    .map(|(i, _)| i + 1),
            );
            let end_offset = offsets
                .get(end as usize)
                .copied()
                .unwrap_or(file.source.len());
            assert_eq!(
                b["source"].as_str().unwrap().as_bytes(),
                &file.source.as_bytes()[offsets[start as usize - 1]..end_offset],
                "{}",
                request.case_id
            );
        }
        assert_eq!(rows["read_missing"]["error"]["code"], -32004);
        assert_eq!(rows["read_path_rejected"]["error"]["code"], -32602);
        assert_eq!(rows["search_invalid_mode"]["error"]["code"], -32602);
        assert!(
            body("read_ambiguous")["candidates"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
        assert!(
            body("code_callees_ambiguous")["candidates"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
        assert!(!rows["code_callees_ambiguity_error"]["error"].is_null());
        assert!(!body("code_callees_samefile_resolved")["callees"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!body("code_callers_samefile_all")["callers"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!body("code_callers_crossfile_all")["callers"]
            .as_array()
            .unwrap()
            .is_empty());
        let resolved = body("code_callees_external_resolved");
        let all = body("code_callees_external_all");
        assert!(
            all["callees"].as_array().unwrap().len()
                > resolved["callees"].as_array().unwrap().len()
        );
        assert!(all["callees"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["resolved"] == false));
        assert!(body("code_callers_empty")["callers"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(body("code_callees_empty")["callees"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            provenance["pointer"]["indexed_commit_oid"],
            provenance["head_oid"]
        );
        for (index, file) in spec.files.iter().enumerate() {
            assert_eq!(
                provenance["sources"][index]["sha256"],
                sha256(file.source.as_bytes())
            );
        }
        // A second module replays the same requests through the actual dispatch
        // entry point; responses (including metadata) are not supplied from disk.
        let module = GraphMcpModule::default();
        let request = &spec.requests[0];
        let replay = dispatch(&module, &root, request).await;
        assert_eq!(replay["body"], rows[&request.case_id]["body"]);
        assert!(
            prepare(&fixture, &root).await.is_err(),
            "overwrote existing corpus"
        );
        fs::write(root.join("src/crlf.rs"), "tampered\n")?;
        assert!(verify(&root).is_err(), "accepted changed source");
        fs::write(
            root.join("src/crlf.rs"),
            spec.files
                .iter()
                .find(|f| f.path == "src/crlf.rs")
                .unwrap()
                .source
                .as_bytes(),
        )?;
        verify(&root)?;
        let artifact = read_current_pointer(&root)?;
        fs::write(artifact.join("unexpected-file"), b"tamper")?;
        assert!(verify(&root).is_err(), "accepted changed artifact");
        Ok(())
    }
}
