//! Opt-in JSON sidecar runner. Reads one request from stdin, writes one response
//! to stdout. Snapshot constructs a fixture artifact in memory, never on disk.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, PathBuf};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spur_graph::panic_evidence::*;
use spur_graph::GraphIndexArtifact;
use spur_solver::service::SolverService;
use spur_solver::types::ModelValue;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    body_id: String,
    parameter_index: u32,
    domain: PanicValueDomain,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SolveInput {
    graph: GraphIndexArtifact,
    sources: Vec<Source>,
    build: PanicBuildConfig,
    facts: PanicSemanticFacts,
    domains: Vec<Domain>,
    limits: PanicAnalysisLimits,
}

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
    CompilerReport {
        graph: GraphIndexArtifact,
        sources: Vec<Source>,
        method_sites: MethodCallSiteEvidenceSet,
        inputs: Vec<Input>,
        identity: CaptureBuildIdentity,
        expected_build_id: String,
        raw_export: String,
        compiler_evidence: CompilerEvidenceSet,
        build: PanicBuildConfig,
        config_binding: PanicCompilerConfigBinding,
    },
    Analyze {
        graph: GraphIndexArtifact,
        sources: Vec<Source>,
        build: PanicBuildConfig,
    },
    Solve {
        #[serde(flatten)]
        input: SolveInput,
    },
    Replay {
        #[serde(flatten)]
        input: SolveInput,
        report: PanicAnalysisReport,
        query_index: usize,
        expected_request_identity: String,
        expected_panic_payload: String,
        execution_timeout_ms: u64,
    },
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
    execute_sync(
        serde_json::from_value::<Request>(request).context("invalid panic evidence request")?,
    )
}

fn execute_sync(request: Request) -> anyhow::Result<Value> {
    match request {
        Request::CompilerReport {
            graph,
            sources,
            method_sites,
            inputs,
            identity,
            expected_build_id,
            raw_export,
            compiler_evidence,
            build,
            config_binding,
        } => {
            let sources = source_refs(&sources);
            let inputs: Vec<_> = inputs
                .iter()
                .map(|i| CaptureInputBytes {
                    path: &i.path,
                    bytes: i.text.as_bytes(),
                })
                .collect();
            let facts = collect_panic_semantics(&graph, &sources, &build)?;
            let calls = collect_rust_call_sites(&graph, &sources)?;
            let report = correlate_panic_compiler(PanicCompilerValidationRequest {
                import: CgaImportRequest {
                    identity: &identity,
                    expected_build_id: &expected_build_id,
                    raw_export: raw_export.as_bytes(),
                    inputs: &inputs,
                    graph: &graph,
                    sources: &sources,
                    method_sites: &method_sites,
                },
                compiler: &compiler_evidence,
                calls: &calls,
                facts: &facts,
                expected_build: &build,
                config_binding: &config_binding,
            })?;
            Ok(json!({"compiler_report":report}))
        }
        Request::Analyze {
            graph,
            sources,
            build,
        } => match collect_panic_semantics(&graph, &source_refs(&sources), &build) {
            Ok(facts) => Ok(json!({"status":"collected", "facts":facts})),
            Err(error) => match error.downcast_ref::<PanicCollectionObligation>() {
                Some(obligation) => Ok(json!({"status":"unknown", "obligation":{
                        "kind":obligation.kind,"detail":obligation.detail}})),
                None => Err(error),
            },
        },
        Request::Solve { .. } | Request::Replay { .. } => {
            anyhow::bail!("solve/replay require execute_async")
        }
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

fn digest(value: &impl Serialize) -> anyhow::Result<String> {
    Ok(sha256_bytes(&serde_json::to_vec(value)?))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn authenticated_facts(input: &SolveInput) -> anyhow::Result<PanicSemanticFacts> {
    let sources = source_refs(&input.sources);
    // This must happen before any domain narrowing: structural validation alone
    // cannot detect missing implicit operations or a producer-forged translation.
    input
        .facts
        .validate_canonical(&input.graph, &sources, &input.build)?;
    let calls = collect_rust_call_sites(&input.graph, &sources)?;
    let mut facts = input.facts.clone();
    let entries: BTreeSet<_> = input.limits.entry_body_ids.iter().collect();
    let mut supplied = BTreeSet::new();
    for domain in &input.domains {
        ensure!(
            entries.contains(&domain.body_id),
            "domain outside explicit roots"
        );
        ensure!(
            supplied.insert((&domain.body_id, domain.parameter_index)),
            "duplicate parameter domain"
        );
        let mut found = false;
        for value in &mut facts.values {
            if value.context.location.body_id == domain.body_id
                && value.origin
                    == (PanicValueOrigin::Parameter {
                        index: domain.parameter_index,
                    })
            {
                // Structural validation below enforces sort and canonical order.
                match (&value.domain, &domain.domain) {
                    (
                        PanicValueDomain::U8 {
                            minimum: a,
                            maximum: b,
                        },
                        PanicValueDomain::U8 {
                            minimum: c,
                            maximum: d,
                        },
                    ) => ensure!(a <= c && d <= b, "domain widening"),
                    (
                        PanicValueDomain::Bool { values: a },
                        PanicValueDomain::Bool { values: b },
                    ) => ensure!(b.iter().all(|v| a.contains(v)), "domain widening"),
                    _ => anyhow::bail!("domain sort mismatch"),
                }
                value.domain = domain.domain.clone();
                found = true;
            }
        }
        ensure!(found, "domain parameter missing from source facts");
    }
    for value in &facts.values {
        if let PanicValueOrigin::Parameter { index } = value.origin {
            if entries.contains(&value.context.location.body_id) {
                ensure!(
                    supplied.contains(&(&value.context.location.body_id, index)),
                    "explicit parameter domain required"
                );
            }
        }
    }
    facts.validate_against(&input.graph, &sources, &calls)?;
    Ok(facts)
}

/// Async operations use the same strict JSON decoder and preserve synchronous
/// callers of snapshot/collect/identity/import/validate/analyze/compiler_report.
pub async fn execute_async(request: Value, service: &SolverService) -> anyhow::Result<Value> {
    match serde_json::from_value::<Request>(request).context("invalid panic evidence request")? {
        Request::Solve { input } => {
            let facts = authenticated_facts(&input)?;
            Ok(json!({"report":analyze_panic_facts(service,&facts,&input.limits).await?}))
        }
        Request::Replay {
            input,
            report,
            query_index,
            expected_request_identity,
            expected_panic_payload,
            execution_timeout_ms,
        } => {
            let facts = authenticated_facts(&input)?;
            let canonical = analyze_panic_facts(service, &facts, &input.limits).await?;
            let query = report
                .queries
                .get(query_index)
                .context("missing replay query")?;
            ensure!(
                query.request_identity == expected_request_identity,
                "replay request identity mismatch"
            );
            // Re-run the exact source-derived requests, and fail closed if this
            // solver selects a different candidate. Retain the original receipt.
            let mut normalized = report.clone();
            ensure!(
                report.queries.len() == canonical.queries.len(),
                "replay query inventory mismatch"
            );
            for (old, new) in normalized.queries.iter_mut().zip(&canonical.queries) {
                let response = old
                    .response
                    .as_ref()
                    .context("replay requires solver response")?;
                ensure!(
                    old.response_digest.as_ref() == Some(&digest(response)?),
                    "replay response digest mismatch"
                );
                ensure!(
                    old.receipt_binding.as_ref()
                        == Some(&digest(&(&old.request_identity, &old.response_digest))?),
                    "replay receipt binding mismatch"
                );
                let saved = service.get_solve_result(
                    response
                        .solve_id
                        .as_deref()
                        .context("replay requires persisted solve")?,
                )?;
                ensure!(
                    saved.request == serde_json::to_value(&old.request)?
                        && saved.result.status == response.status
                        && saved.result.model == response.model,
                    "replay persisted receipt mismatch"
                );
                ensure!(
                    new.response
                        .as_ref()
                        .is_some_and(|r| r.status == response.status && r.model == response.model),
                    "replay solver model changed"
                );
                old.response = new.response.clone();
                old.response_digest = new.response_digest.clone();
                old.receipt_binding = new.receipt_binding.clone();
            }
            ensure!(
                normalized == canonical,
                "replay differs from source/build/request-bound analysis"
            );
            ensure!(
                query.outcome == PanicAnalysisOutcome::CandidateWitness
                    && query.obligations.is_empty(),
                "replay requires complete bounded candidate"
            );
            let replay = replay_fixture(
                &input,
                &report,
                query,
                &expected_panic_payload,
                execution_timeout_ms,
            )
            .await?;
            Ok(json!({"replay":replay}))
        }
        other => execute_sync(other),
    }
}

fn compiler_environment() -> BTreeMap<String, String> {
    // No compiler wrapper, RUSTFLAGS, injected library or ambient cfg arguments.
    [
        "PATH",
        "HOME",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "CARGO_HOME",
        "TMPDIR",
        "SDKROOT",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_owned(), v)))
    .collect()
}

async fn run_process(
    program: &std::path::Path,
    args: &[String],
    dir: &std::path::Path,
    environment: &BTreeMap<String, String>,
    timeout_ms: u64,
) -> anyhow::Result<std::process::Output> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(environment)
        .kill_on_drop(true);
    tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        command.output(),
    )
    .await
    .context("fixture compiler/execution timeout; no reproduction established")?
    .context("fixture compiler/execution failed")
}

async fn replay_fixture(
    input: &SolveInput,
    report: &PanicAnalysisReport,
    query: &PanicAnalysisQuery,
    expected_payload: &str,
    timeout_ms: u64,
) -> anyhow::Result<Value> {
    ensure!(
        timeout_ms > 0 && timeout_ms <= spur_solver::types::MAX_TIMEOUT_MS,
        "invalid execution budget"
    );
    ensure!(
        input.sources.len() == 1,
        "replay supports one standalone source file"
    );
    ensure!(
        input.build.panic_strategy == PanicStrategy::Unwind
            && input.build.codegen_flags.is_empty()
            && input.build.features.is_empty(),
        "replay supports unwind with explicit check modes and no additional flags/features"
    );
    let body = input
        .facts
        .bodies
        .iter()
        .find(|b| b.body_id == query.entry_body_id)
        .context("missing replay body")?;
    ensure!(
        body.kind == RustBodyKind::Function,
        "replay requires top-level synchronous function"
    );
    ensure!(
        body.file_path == input.sources[0].file_path,
        "replay source inventory mismatch"
    );
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser
        .parse(&input.sources[0].text, None)
        .context("replay source parse")?;
    let mut cursor = tree.root_node().walk();
    let function = tree
        .root_node()
        .named_children(&mut cursor)
        .find(|n| {
            n.kind() == "function_item"
                && n.child_by_field_name("body")
                    .is_some_and(|b| [b.start_byte(), b.end_byte()] == body.byte_range)
        })
        .context("replay entry is not a top-level function")?;
    let name = function
        .child_by_field_name("name")
        .context("missing replay function name")?
        .utf8_text(input.sources[0].text.as_bytes())?;
    let parameters = function
        .child_by_field_name("parameters")
        .context("missing replay signature")?;
    let mut cursor = parameters.walk();
    let parameters: Vec<_> = parameters.named_children(&mut cursor).collect();
    let candidate = query.candidate.as_ref().context("missing candidate")?;
    ensure!(
        !candidate.runtime_reproduced,
        "solver cannot supply reproduced evidence"
    );
    let mut inputs = query.inputs.clone();
    inputs.sort_by_key(|i| i.parameter_index);
    ensure!(
        inputs.len() == parameters.len(),
        "replay requires all signature parameters in the bounded model"
    );
    let mut arguments = Vec::new();
    for (index, (value, parameter)) in inputs.iter().zip(&parameters).enumerate() {
        ensure!(
            value.parameter_index == index as u32,
            "replay parameter inventory mismatch"
        );
        let ty = parameter
            .child_by_field_name("type")
            .context("unsupported replay parameter")?
            .utf8_text(input.sources[0].text.as_bytes())?;
        arguments.push(match (ty, candidate.inputs.get(&value.variable)) {
            ("u8", Some(ModelValue::Int(x))) if (0..=255).contains(x) => format!("{x}u8"),
            ("bool", Some(ModelValue::Bool(x))) => x.to_string(),
            _ => anyhow::bail!("replay requires concrete primitive u8/bool parameters"),
        });
    }
    let dir = tempfile::tempdir()?;
    let environment = compiler_environment();
    let version = run_process(
        std::path::Path::new("rustc"),
        &["-vV".into()],
        dir.path(),
        &environment,
        timeout_ms,
    )
    .await?;
    ensure!(
        version.status.success() && String::from_utf8(version.stdout)? == input.build.rustc_verbose,
        "replay compiler identity mismatch"
    );
    let host = input
        .build
        .rustc_verbose
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .context("missing compiler host")?;
    ensure!(
        host == input.build.target,
        "replay target must be the native compiler host"
    );
    let settings = vec![
        "--edition".into(),
        serde_json::to_value(input.build.edition)?
            .as_str()
            .unwrap()
            .into(),
        "--target".into(),
        input.build.target.clone(),
        "-C".into(),
        "opt-level=0".into(),
        "-C".into(),
        format!("overflow-checks={}", input.build.overflow_checks),
        "-C".into(),
        format!("debug-assertions={}", input.build.debug_assertions),
        "-C".into(),
        "panic=unwind".into(),
    ];
    let mut cfg_args = settings.clone();
    cfg_args.extend(["--print".into(), "cfg".into()]);
    let cfg = run_process(
        std::path::Path::new("rustc"),
        &cfg_args,
        dir.path(),
        &environment,
        timeout_ms,
    )
    .await?;
    ensure!(cfg.status.success(), "replay compiler cfg query failed");
    let mut actual_cfg = Vec::new();
    for line in String::from_utf8(cfg.stdout)?.lines() {
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key.to_owned(), Some(serde_json::from_str(value)?)),
            None => (line.to_owned(), None),
        };
        actual_cfg.push(PanicCfgAtom { key, value });
    }
    actual_cfg.sort();
    ensure!(
        actual_cfg == input.build.cfg,
        "replay compiler cfg differs from explicit build"
    );
    std::fs::write(dir.path().join("fixture.rs"), &input.sources[0].text)?;
    let harness = format!(
        r#"include!("fixture.rs");
fn main() {{
    std::panic::set_hook(Box::new(|_|{{}}));
    match std::panic::catch_unwind(|| {{ let _ = {name}({args}); }}) {{
        Ok(()) => println!("RETURNED"),
        Err(error) => {{
            if let Some(text)=error.downcast_ref::<&str>() {{ println!("PANIC:{{}}",text); }}
            else if let Some(text)=error.downcast_ref::<String>() {{ println!("PANIC:{{}}",text); }}
            else {{ println!("NON_STRING_PANIC"); }}
        }}
    }}
}}
"#,
        args = arguments.join(",")
    );
    std::fs::write(dir.path().join("main.rs"), &harness)?;
    let mut compile_args = settings;
    compile_args.extend(["main.rs".into(), "-o".into(), "fixture-bin".into()]);
    let compiled = run_process(
        std::path::Path::new("rustc"),
        &compile_args,
        dir.path(),
        &environment,
        timeout_ms,
    )
    .await?;
    ensure!(
        compiled.status.success(),
        "fixture compilation failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let binary = dir.path().join("fixture-bin");
    let binary_digest = sha256_bytes(&std::fs::read(&binary)?);
    let execution = run_process(&binary, &[], dir.path(), &environment, timeout_ms).await?;
    let stdout = String::from_utf8(execution.stdout)?;
    let stderr = String::from_utf8(execution.stderr)?;
    let actual_payload = stdout
        .strip_prefix("PANIC:")
        .and_then(|s| s.strip_suffix('\n'));
    let reproduced = execution.status.success() && actual_payload == Some(expected_payload);
    let outcome = if reproduced {
        "reproduced"
    } else if actual_payload.is_some() {
        "panic_payload_mismatch"
    } else if stdout == "RETURNED\n" {
        "returned"
    } else {
        "execution_error"
    };
    let mut record = json!({"schema_version":"panic-fixture-replay-v1","scope":"standalone_fixture",
        "graph_content_hash":report.graph_content_hash,"sources":report.sources,"build_hash":report.build_hash,
        "fact_digest":report.fact_digest,"request_digest":query.request_digest,"request_identity":query.request_identity,
        "receipt_binding":query.receipt_binding,"candidate":candidate,"expected_panic_payload":expected_payload,
        "actual_panic_payload":actual_payload,"outcome":outcome,"runtime_reproduced":reproduced,
        "compiler_args":compile_args,"compiler_environment":environment,"execution_timeout_ms":timeout_ms,
        "harness_digest":sha256_bytes(harness.as_bytes()),"binary_digest":binary_digest,
        "exit_code":execution.status.code(),"stdout":stdout,"stderr":stderr,
        "limitations":["Only this source and generated fixture harness ran; no arbitrary workspace binary or general safety is established.",
            "Hashes bind local observations, not producer authenticity; source and compiler are trusted local inputs."]});
    record["replay_digest"] = json!(digest(&record)?);
    Ok(record)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let service = SolverService::new().with_repo_root(std::env::current_dir()?);
    let response = execute_async(serde_json::from_str(&input)?, &service).await?;
    serde_json::to_writer(std::io::stdout().lock(), &response)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}
