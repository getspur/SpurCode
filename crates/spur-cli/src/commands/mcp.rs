//! `spur mcp`, `spur graph mcp`, `spur analyst mcp`, `spur solver mcp`, and
//! `spur context mcp` — standalone MCP servers launched directly from the
//! `spur` CLI.
//!
//! Each builds a [`spur_mcp::ToolRegistry`] from one or more domain
//! `ToolModule`s, wraps it in a [`RegistryServerHandler`], and serves it over
//! stdio so an MCP client (Claude Code, OpenCode, …) can launch it via
//! `command`/`args`:
//!
//! ```jsonc
//! { "mcpServers": {
//!     "spur-graph":   { "command": "spur", "args": ["graph", "mcp"] },
//!     "spur-analyst": { "command": "spur", "args": ["analyst", "mcp"] },
//!     "spur-solver":  { "command": "spur", "args": ["solver", "mcp"] },
//!     "spur-context": { "command": "spur", "args": ["context", "mcp", "--url", "..."] },
//!     "spur":         { "command": "spur", "args": ["mcp"] }
//! }}
//! ```
//!
//! Pass `--root <path>` in any of those `args` arrays to bind every tool call
//! to a specific worktree for the server lifetime. If `--root` is omitted,
//! `SPUR_WORKTREE` is used as a fallback; if neither is set, tools keep the
//! existing behavior of resolving from the MCP client launch directory.
//!
//! The bundled `spur mcp` server keeps repository and index queries read-only.
//! Its local-project add/remove tools mutate only user catalog configuration;
//! orchestration tools (delegation, plan, review) stay bound to the
//! TUI/orchestrator because they need an active brain session and PM service.
//!
//! Logging: these commands run in non-TUI mode, so `init_tracing` directs all
//! log output to **stderr**, keeping stdout free for the JSON-RPC stream.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use spur_analyst::embedding::OwnedEmbeddingRuntime;
use spur_analyst::mcp::AnalystMcpModule;
use spur_core::mcp::{ContextServiceAuth, ContextServiceClient, LocalProjectMcpComposition};
use spur_graph::mcp::{GraphMcpDeps, GraphMcpModule};
use spur_mcp::{serve_stdio_server, RegistryServerHandler, ToolRegistry};
use spur_solver::{mcp::SolverMcpModule, service::SolverService};
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
#[path = "mcp_stdin.rs"]
mod mcp_stdin;

const SPUR_WORKTREE_ENV: &str = "SPUR_WORKTREE";

const GRAPH_INSTRUCTIONS: &str =
    "Tree-sitter code-graph query tools over the current worktree: resolve/search/read symbols, \
     list callers/callees, map neighborhoods, and trace symbol history. Graph-first code \
     navigation — prefer these over grep/glob. Repository and index query operations are read-only; \
     local_project_add and local_project_remove mutate only user catalog configuration. Register an \
     already-indexed local Git project once and pass its name as project; registration validates but \
     does not index. external_* tools are the separate hosted package/revision surface.";

const ANALYST_INSTRUCTIONS: &str =
    "DuckDB-backed analytics over the .spur/analyst.duckdb index: knowledge_context_pack for \
     one-shot oriented evidence, and doc_navigate for documentation section search. Use these for \
     ranked/aggregated/path-shaped answers over code and docs. Repository and index query operations \
     are read-only; local_project_add and local_project_remove mutate only user catalog configuration. \
     Register an already-indexed local Git project once and pass its name as project; registration \
     validates but does not index. external_* tools are the separate hosted package/revision surface.";

const SOLVER_INSTRUCTIONS: &str =
    "Z3-backed constraint workbench: navigate the mathematical rule catalog with solve_rule_spec, \
     verify or synthesize catalog models with solve_rules, preflight and execute generic typed \
     constraints with solve_constraint_spec / solve_constraint_check / solve_constraints, escape to \
     raw SMT with solve_smt only when the typed surface cannot express the theory, and reload \
     persisted results with get_solve_result. Preserve raw solver statuses — unknown and timeout \
     are never proof outcomes.";

const CONTEXT_INSTRUCTIONS: &str =
    "Cloud-backed external code-context tools (external_*): search/read indexed packages, inspect \
     callers/callees, query external knowledge context, and manage external indexes.";

const BUNDLED_INSTRUCTIONS: &str =
    "SPUR standalone query surface: code-graph tools (code_*) plus DuckDB analyst tools \
     (knowledge_context_pack*, doc_navigate). Repository and index query operations are read-only; \
     local_project_add and local_project_remove mutate only user catalog configuration. For \
     orchestration (delegation, plans, review) run the SPUR TUI. Register an already-indexed local \
     Git project once and pass its name as project; registration validates but does not index. \
     external_* tools remain the separate hosted package/revision surface.";

fn resolve_mcp_worktree_root(root: Option<PathBuf>) -> Result<Option<PathBuf>> {
    let root = root.or_else(|| {
        std::env::var_os(SPUR_WORKTREE_ENV).and_then(|value| {
            if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            }
        })
    });
    root.map(|root| {
        root.canonicalize()
            .with_context(|| format!("failed to canonicalize root `{}`", root.display()))
    })
    .transpose()
}

async fn with_mcp_worktree_scope<F, T>(root: Option<PathBuf>, future: F) -> Result<T>
where
    F: Future<Output = T>,
{
    match resolve_mcp_worktree_root(root)? {
        Some(root) => Ok(spur_graph::mcp::with_worktree_root_for_request(root, future).await),
        None => Ok(future.await),
    }
}

fn local_project_composition() -> LocalProjectMcpComposition {
    LocalProjectMcpComposition::from_environment()
}

fn graph_server_registry(local_projects: &LocalProjectMcpComposition) -> Result<ToolRegistry> {
    Ok(ToolRegistry::builder()
        .with(local_projects.catalog_module())?
        .with(GraphMcpModule::with_local_projects(
            GraphMcpDeps::default(),
            local_projects.resolver(),
        ))?
        .with_alias("code_search", "code_symbol_search")?
        .build())
}

fn analyst_server_registry(
    local_projects: &LocalProjectMcpComposition,
    runtime: OwnedEmbeddingRuntime,
) -> Result<ToolRegistry> {
    Ok(ToolRegistry::builder()
        .with(local_projects.catalog_module())?
        .with(
            AnalystMcpModule::with_local_projects_for_analyst_server(local_projects.resolver())
                .with_embedding_runtime(runtime),
        )?
        .build())
}

fn bundled_server_registry(
    local_projects: &LocalProjectMcpComposition,
    runtime: OwnedEmbeddingRuntime,
) -> Result<ToolRegistry> {
    Ok(ToolRegistry::builder()
        .with(local_projects.catalog_module())?
        .with(GraphMcpModule::with_local_projects(
            GraphMcpDeps::default(),
            local_projects.resolver(),
        ))?
        .with(
            AnalystMcpModule::with_local_projects(local_projects.resolver())
                .with_embedding_runtime(runtime),
        )?
        .with_alias("code_search", "code_symbol_search")?
        .build())
}

/// `spur graph mcp` — standalone code-graph MCP server (the 9 `code_*` tools).
///
/// `root` is the optional `--root <path>` override. When absent, `SPUR_WORKTREE`
/// is honored before falling back to the MCP client launch directory.
pub async fn run_graph_server(root: Option<PathBuf>) -> Result<()> {
    let local_projects = local_project_composition();
    let registry = graph_server_registry(&local_projects)?;
    let handler = RegistryServerHandler::new(registry, "spur-graph-mcp", GRAPH_INSTRUCTIONS);
    Box::pin(with_mcp_worktree_scope(root, serve_stdio_server(handler))).await?
}

/// `spur analyst mcp` — standalone analyst MCP server
/// (`knowledge_context_pack`, `knowledge_context_pack_2`, `doc_navigate`).
///
/// `root` is the optional `--root <path>` override. When absent, `SPUR_WORKTREE`
/// is honored before falling back to the MCP client launch directory.
pub async fn run_analyst_server(root: Option<PathBuf>) -> Result<()> {
    with_command_embedding_runtime(|runtime, cancel| async move {
        let local_projects = local_project_composition();
        let registry = analyst_server_registry(&local_projects, runtime)?;
        let handler =
            RegistryServerHandler::new(registry, "spur-analyst-mcp", ANALYST_INSTRUCTIONS);
        Box::pin(with_mcp_worktree_scope(
            root,
            serve_command_stdio(handler, cancel),
        ))
        .await?
    })
    .await
}

async fn with_command_embedding_runtime<F, Fut>(operation: F) -> Result<()>
where
    F: FnOnce(OwnedEmbeddingRuntime, CancellationToken) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Register both ordinary Unix signals before warm can start a child. Keep
    // the listeners alive until the owned runtime has finished shutting down.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install command termination listener")?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("install command interrupt listener")?;

    let runtime = OwnedEmbeddingRuntime::new();
    let cancel = CancellationToken::new();
    let result = {
        let work = async {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                warm = runtime.warm() => if let Err(error) = warm {
                    tracing::warn!(%error, "command embedding warm failed");
                },
            }
            operation(runtime.clone(), cancel.clone()).await
        };
        tokio::pin!(work);
        #[cfg(unix)]
        let signal = async {
            tokio::select! {
                _ = terminate.recv() => Ok::<_, anyhow::Error>("SIGTERM"),
                _ = interrupt.recv() => Ok("SIGINT"),
            }
        };
        #[cfg(not(unix))]
        let signal = async {
            tokio::signal::ctrl_c()
                .await
                .context("command interrupt listener failed")?;
            Ok::<_, anyhow::Error>("interrupt")
        };
        tokio::select! {
            biased;
            signal = signal => {
                cancel.cancel();
                let (work_result, shutdown_result) = tokio::join!(&mut work, runtime.shutdown());
                shutdown_result.context("command embedding shutdown failed")?;
                work_result?;
                return signal.and_then(|name| Err(anyhow::anyhow!("command terminated by {name}")));
            },
            result = &mut work => result,
        }
    };
    runtime
        .shutdown()
        .await
        .context("command embedding shutdown failed")?;
    result
}

async fn serve_command_stdio(
    handler: RegistryServerHandler,
    cancel: CancellationToken,
) -> Result<()> {
    #[cfg(unix)]
    let stdin = mcp_stdin::command_stdin()?;
    #[cfg(not(unix))]
    let stdin = tokio::io::stdin();
    let running =
        rmcp::service::serve_server_with_ct(handler, (stdin, tokio::io::stdout()), cancel.clone())
            .await;
    let running = match running {
        Ok(running) => running,
        Err(_) if cancel.is_cancelled() => return Ok(()),
        Err(error) => return Err(error).context("failed to start stdio MCP server"),
    };
    running
        .waiting()
        .await
        .context("stdio MCP server exited unexpectedly")?;
    Ok(())
}

fn context_server_registry(url: String, auth: ContextServiceAuth) -> Result<ToolRegistry> {
    Ok(ToolRegistry::builder()
        .with(ContextServiceClient::new(url, auth)?)?
        .build())
}

fn legacy_context_server_registry(url: String, token: String) -> Result<ToolRegistry> {
    Ok(ToolRegistry::builder()
        .with(ContextServiceClient::with_optional_token(url, Some(token))?)?
        .build())
}

/// `spur solver mcp` — standalone solver MCP server (the 7 `solve_*` tools).
///
/// `root` is the optional `--root <path>` override. When absent, `SPUR_WORKTREE`
/// is honored before falling back to the MCP client launch directory. The
/// resolved root scopes persisted solve artifacts (`.spur/solver/`), mirroring
/// the worker MCP's per-repo-root solver services. The Z3 process is lazy: it
/// spawns on first tool use, so an idle server costs only the stdio process.
pub async fn run_solver_server(root: Option<PathBuf>) -> Result<()> {
    let resolved = solver_service_root(root)?;
    let registry = solver_server_registry(resolved.as_deref())?;
    let handler = RegistryServerHandler::new(registry, "spur-solver-mcp", SOLVER_INSTRUCTIONS);
    serve_stdio_server(handler).await
}

/// Resolve the solver service root: explicit `--root`, then `SPUR_WORKTREE`,
/// then the MCP client launch directory, canonicalized.
///
/// Unlike the graph/analyst servers, which scope each request through
/// [`with_mcp_worktree_scope`] after startup, [`SolverService`] binds its
/// artifact store at construction, so the documented client-launch-directory
/// fallback must be applied here — otherwise `persist: true` fails with
/// "solver persistence requires an explicit repository root" for servers
/// started from a portable `.pi/mcp.json` entry without `--root`.
fn solver_service_root(root: Option<PathBuf>) -> Result<Option<PathBuf>> {
    let resolved = resolve_mcp_worktree_root(root)?;
    if resolved.is_some() {
        return Ok(resolved);
    }
    let launch_dir = std::env::current_dir()
        .with_context(|| "failed to read MCP client launch directory".to_string())?;
    let launch_dir = launch_dir.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize MCP client launch directory `{}`",
            launch_dir.display()
        )
    })?;
    Ok(Some(launch_dir))
}

fn solver_server_registry(root: Option<&std::path::Path>) -> Result<ToolRegistry> {
    let service = match root {
        Some(root) => SolverService::new().with_repo_root(root),
        None => SolverService::new(),
    };
    Ok(ToolRegistry::builder()
        .with(SolverMcpModule::new(Arc::new(service)))?
        .build())
}

/// `spur context mcp` — standalone external code-context MCP server.
///
/// Exposes the 7 `external_*` tools (external_code_search, external_code_read,
/// external_code_callers, external_code_callees, external_knowledge_context,
/// external_index, external_index_status) as a spec-compliant MCP stdio server.
/// Tools proxy to the cloud spur-context-service Lambda.
pub async fn run_context_server(url: String, auth: ContextServiceAuth) -> Result<()> {
    let registry = context_server_registry(url, auth)?;
    let handler = RegistryServerHandler::new(registry, "spur-context-mcp", CONTEXT_INSTRUCTIONS);
    serve_stdio_server(handler).await
}

/// Runs the context MCP proxy while preserving the configured legacy route.
pub async fn run_legacy_context_server(url: String, token: String) -> Result<()> {
    let registry = legacy_context_server_registry(url, token)?;
    let handler = RegistryServerHandler::new(registry, "spur-context-mcp", CONTEXT_INSTRUCTIONS);
    serve_stdio_server(handler).await
}

/// `spur mcp` — bundled MCP server with read-only repository/index queries and
/// user catalog configuration tools.
///
/// `root` is the optional `--root <path>` override. When absent, `SPUR_WORKTREE`
/// is honored before falling back to the MCP client launch directory.
pub async fn run_bundled_server(root: Option<PathBuf>) -> Result<()> {
    with_command_embedding_runtime(|runtime, cancel| async move {
        let local_projects = local_project_composition();
        let registry = bundled_server_registry(&local_projects, runtime)?;
        let handler = RegistryServerHandler::new(registry, "spur-mcp", BUNDLED_INSTRUCTIONS);
        Box::pin(with_mcp_worktree_scope(
            root,
            serve_command_stdio(handler, cancel),
        ))
        .await?
    })
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use spur_core::mcp::LocalProjectMcpComposition;
    use spur_graph::{
        artifact_from_facts, build_facts, write_artifact_parquet, write_current_pointer,
        WriteOptions,
    };
    use spur_mcp::local_projects::LocalProjectCatalogStore;
    use spur_mcp::{ServerKind, ToolAuthority, ToolCallContext};

    fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("missing function signature: {signature}"));
        let tail = &source[start..];
        let next_fn = tail
            .find("\npub async fn ")
            .filter(|offset| *offset > 0)
            .unwrap_or(tail.len());
        &tail[..next_fn]
    }

    #[test]
    fn solver_registry_exposes_the_seven_solve_tools() {
        let registry = super::solver_server_registry(None).expect("solver registry");
        let mut names: Vec<String> = registry
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "get_solve_result",
                "solve_constraint_check",
                "solve_constraint_spec",
                "solve_constraints",
                "solve_rule_spec",
                "solve_rules",
                "solve_smt",
            ]
        );
    }

    #[test]
    fn solver_root_falls_back_to_client_launch_directory() {
        let explicit = tempfile::tempdir().expect("tempdir");
        let explicit = explicit
            .path()
            .canonicalize()
            .expect("canonical explicit root");
        assert_eq!(
            super::solver_service_root(Some(explicit.clone())).unwrap(),
            Some(explicit),
            "explicit --root wins unchanged"
        );

        let cwd = std::env::current_dir()
            .expect("current dir")
            .canonicalize()
            .expect("canonical cwd");
        assert_eq!(
            super::solver_service_root(None).unwrap(),
            Some(cwd),
            "omitted root falls back to the canonicalized client launch directory"
        );
    }

    #[test]
    fn standalone_query_servers_pre_warm_kcp_embeddings() {
        let source = include_str!("mcp.rs");

        for signature in [
            "pub async fn run_analyst_server",
            "pub async fn run_bundled_server",
        ] {
            let body = function_body(source, signature);
            assert!(
                body.contains("with_command_embedding_runtime("),
                "{signature} must pre-warm knowledge_context_pack embeddings"
            );
        }
        assert!(source.contains("runtime.warm().await"));
    }

    #[tokio::test]
    async fn worktree_scope_wraps_entire_server_future() {
        let root = std::env::current_dir()
            .expect("current dir")
            .canonicalize()
            .expect("canonical root");
        let expected = spur_graph::resolve_worktree_root_from(root.clone());

        let scoped = super::with_mcp_worktree_scope(Some(root), async {
            tokio::task::yield_now().await;
            spur_graph::mcp::scoped_worktree_root()
        })
        .await
        .expect("scoped server future");

        assert_eq!(scoped, Some(expected));
    }

    #[tokio::test]
    async fn omitted_project_dispatches_registry_call_against_outer_root() {
        let repo = tempfile::tempdir().expect("repository tempdir");
        std::fs::create_dir_all(repo.path().join("src")).expect("create source directory");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn selected_by_outer_root() {}\n",
        )
        .expect("write source");
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "SPUR Test"][..],
            &["add", "src/lib.rs"][..],
            &["commit", "-q", "-m", "fixture"][..],
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        }
        let (facts, _) = build_facts(repo.path(), None).expect("build graph facts");
        let artifact = artifact_from_facts(&facts, repo.path()).expect("build graph artifact");
        let artifact_dir = write_artifact_parquet(
            &artifact,
            &repo.path().join(".spur/graph"),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write graph artifact");
        write_current_pointer(repo.path(), &artifact_dir).expect("write graph pointer");

        let catalog = tempfile::tempdir().expect("catalog tempdir");
        let local_projects = LocalProjectMcpComposition::new(LocalProjectCatalogStore::new(
            catalog.path().join("projects.toml"),
        ));
        let registry = super::graph_server_registry(&local_projects).expect("graph registry");
        let response = super::with_mcp_worktree_scope(Some(repo.path().to_path_buf()), async {
            registry
                .call_json_tool(
                    ToolCallContext::new(ServerKind::Brain, ToolAuthority::Brain, None, None),
                    "code_symbol_search",
                    json!({"query": "selected_by_outer_root", "mode": "exact"}),
                )
                .await
        })
        .await
        .expect("outer worktree scope");
        let response = serde_json::to_value(response).expect("serialize response");
        let body = response["result"]["content"][0]["text"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| panic!("unexpected registry response: {response:#?}"));
        assert_eq!(
            body["candidates"][0]["entity_name"],
            "selected_by_outer_root"
        );
        assert!(body.get("project").is_none());
    }

    #[test]
    fn context_server_registry_exposes_external_tools() {
        let registry = super::context_server_registry(
            "https://context.example.test".to_owned(),
            super::ContextServiceAuth::None,
        )
        .expect("context registry should build");
        let tools = registry.list_tools();
        let names: std::collections::BTreeSet<_> =
            tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(tools.len(), 8);
        assert!(names.contains("external_catalog"));
        assert!(names.contains("external_code_search"));
        assert!(names.contains("external_code_read"));
        assert!(names.contains("external_code_callers"));
        assert!(names.contains("external_code_callees"));
        assert!(names.contains("external_knowledge_context"));
        assert!(names.contains("external_index"));
        assert!(names.contains("external_index_status"));
    }

    #[test]
    fn standalone_query_registries_expose_named_local_projects() {
        let catalog = tempfile::tempdir().expect("catalog tempdir");
        let local_projects = LocalProjectMcpComposition::new(LocalProjectCatalogStore::new(
            catalog.path().join("projects.toml"),
        ));
        let graph = super::graph_server_registry(&local_projects).expect("graph registry");
        let analyst = super::analyst_server_registry(
            &local_projects,
            spur_analyst::embedding::OwnedEmbeddingRuntime::new(),
        )
        .expect("analyst registry");
        let bundled = super::bundled_server_registry(
            &local_projects,
            spur_analyst::embedding::OwnedEmbeddingRuntime::new(),
        )
        .expect("bundled registry");

        for (name, registry) in [
            ("graph", &graph),
            ("analyst", &analyst),
            ("bundled", &bundled),
        ] {
            let tools = registry.list_tools();
            for management in [
                "local_project_add",
                "local_project_list",
                "local_project_remove",
            ] {
                assert!(
                    tools.iter().any(|tool| tool.name == management),
                    "{name} registry missing {management}"
                );
            }
        }

        assert_project_schema(&graph, "code_symbol_search");
        assert_project_schema(&analyst, "query");
        assert_project_schema(&bundled, "code_symbol_search");
        assert_project_schema(&bundled, "knowledge_context_pack_2");
        assert_eq!(
            bundled
                .canonical_name_for_call("code_search")
                .expect("bundled alias"),
            "code_symbol_search"
        );
    }

    #[cfg(feature = "embed")]
    #[tokio::test]
    async fn analyst_registry_reuses_command_owned_runtime() {
        const CHILD: &str = "SPUR_S6_CLI_HOST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os("SPUR_S6_CLI_ROOT").unwrap());
            let projects = LocalProjectMcpComposition::new(LocalProjectCatalogStore::new(
                root.join("catalog.toml"),
            ));
            let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
            let registry = super::analyst_server_registry(&projects, runtime.clone()).unwrap();
            for query in ["cli first", "cli second"] {
                let response = super::with_mcp_worktree_scope(Some(root.clone()), async {
                    registry
                        .call_json_tool(
                            ToolCallContext::new(
                                ServerKind::Brain,
                                ToolAuthority::Brain,
                                None,
                                None,
                            ),
                            "knowledge_context_pack_2",
                            json!({"query": query}),
                        )
                        .await
                })
                .await
                .unwrap();
                assert!(
                    response.error.is_some(),
                    "empty fixture must return database error"
                );
            }
            runtime.shutdown().await.expect("joined command shutdown");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".spur")).unwrap();
        drop(duckdb::Connection::open(dir.path().join(".spur/analyst.duckdb")).unwrap());
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
        let config = json!({
            "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let log_path = dir.path().join("worker.log");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::mcp::tests::analyst_registry_reuses_command_owned_runtime",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("SPUR_S6_CLI_ROOT", dir.path())
            .env("SPUR_GRAPH_TEST_WORKER_LOG", &log_path)
            .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let log = std::fs::read_to_string(log_path).unwrap();
        assert_eq!(
            log.lines().filter(|line| *line == "service").count(),
            1,
            "{log}"
        );
        assert_eq!(
            log.lines()
                .filter(|line| line.starts_with("embed "))
                .count(),
            2,
            "{log}"
        );
        assert_eq!(log.lines().last(), Some("shutdown"), "{log}");
    }

    #[test]
    fn analyst_only_registry_selects_same_surface_followup_policy() {
        let source = include_str!("mcp.rs");
        let analyst_start = source
            .find("fn analyst_server_registry")
            .expect("analyst registry function");
        let bundled_start = source
            .find("fn bundled_server_registry")
            .expect("bundled registry function");
        let analyst_body = &source[analyst_start..bundled_start];

        assert!(
            analyst_body.contains("with_local_projects_for_analyst_server"),
            "standalone analyst composition must suppress graph-only follow-ups"
        );
    }

    #[test]
    fn standalone_instructions_explain_local_registration_boundary() {
        for (name, instructions) in [
            ("graph", super::GRAPH_INSTRUCTIONS),
            ("analyst", super::ANALYST_INSTRUCTIONS),
            ("bundled", super::BUNDLED_INSTRUCTIONS),
        ] {
            assert!(instructions.contains("already-indexed"), "{name}");
            assert!(instructions.contains("does not index"), "{name}");
            assert!(instructions.contains("external_*"), "{name}");
            assert!(
                instructions.contains("Repository and index query operations are read-only"),
                "{name}"
            );
            assert!(instructions.contains("local_project_add"), "{name}");
            assert!(instructions.contains("local_project_remove"), "{name}");
            assert!(
                instructions.contains("user catalog configuration"),
                "{name}"
            );
        }
    }

    fn assert_project_schema(registry: &spur_mcp::ToolRegistry, tool_name: &str) {
        let tool = registry
            .list_tools()
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("missing {tool_name}"));
        assert_eq!(tool.input_schema["properties"]["project"]["type"], "string");
    }
}
