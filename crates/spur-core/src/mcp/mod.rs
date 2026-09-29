pub(crate) mod catalog;
pub mod context_service;
pub mod delegation;
pub mod families;
pub mod local_projects;
pub mod plan;
pub mod review_verdict;
pub mod signals;
pub mod skills_catalog;
pub mod worker;
pub(crate) mod worker_evidence;

pub use families::{resolve_mcp_families, McpFamily, McpFamilyError};

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub use context_service::{ContextServiceAuth, ContextServiceClient};
pub use local_projects::{IndexedLocalProjectValidator, LocalProjectMcpComposition};
use spur_acp::config::ContextServiceConfig;

const WORKER_DENIED_TOOL_CALLS: &[&str] = &[
    "delegate_to_worker",
    "delegate_parallel",
    "check_delegation_status",
    "cancel_delegation",
    "list_available_workers",
    "update_issue",
    "create_issue",
    "add_dependency",
    "create_pr",
    "merge_plan",
    "resume_plan",
    "force_reclaim_plan",
    "submit_plan",
    "spur_loop_doctor",
    "submit_loop",
    "execute_epic",
    "get_reconciler_status",
    "get_loop_status",
    "pause_loop",
    "resume_loop",
    "kill_loop",
    "set_loop_autonomy",
    "preview_task_base",
    "plan_truncate_and_restart",
    "recover_orphaned_dispatch",
    "review_task",
    "submit_plan_mutation",
    "graph_triage",
    "graph_plan",
    "graph_insights",
    "graph_alerts",
    "graph_subgraph",
];

pub fn brain_tool_registry(
    delegation_deps: delegation::DelegationMcpDeps,
    plan_deps: plan::PlanMcpDeps,
    signal_deps: signals::SignalMcpDeps,
    context_service_config: &ContextServiceConfig,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let local_projects = delegation_deps.local_projects().clone();
    brain_tool_registry_with_local_projects(
        delegation_deps,
        plan_deps,
        signal_deps,
        context_service_config,
        &local_projects,
    )
}

/// Build the brain tool registry using layered configuration rooted at `repo_root`.
pub fn brain_tool_registry_for_repo_root(
    delegation_deps: delegation::DelegationMcpDeps,
    plan_deps: plan::PlanMcpDeps,
    signal_deps: signals::SignalMcpDeps,
    context_service_config: &ContextServiceConfig,
    repo_root: &Path,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let local_projects = delegation_deps.local_projects().clone();
    brain_tool_registry_with_local_projects_and_repo_root(
        delegation_deps,
        plan_deps,
        signal_deps,
        context_service_config,
        &local_projects,
        Some(repo_root),
    )
}

pub(crate) fn brain_tool_registry_with_local_projects(
    delegation_deps: delegation::DelegationMcpDeps,
    plan_deps: plan::PlanMcpDeps,
    signal_deps: signals::SignalMcpDeps,
    context_service_config: &ContextServiceConfig,
    local_projects: &LocalProjectMcpComposition,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    brain_tool_registry_with_local_projects_and_repo_root(
        delegation_deps,
        plan_deps,
        signal_deps,
        context_service_config,
        local_projects,
        None,
    )
}

fn brain_tool_registry_with_local_projects_and_repo_root(
    delegation_deps: delegation::DelegationMcpDeps,
    plan_deps: plan::PlanMcpDeps,
    signal_deps: signals::SignalMcpDeps,
    context_service_config: &ContextServiceConfig,
    local_projects: &LocalProjectMcpComposition,
    repo_root: Option<&Path>,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let mut builder = spur_mcp::ToolRegistry::builder()
        .with(delegation::DelegationMcpModule::new(delegation_deps))?
        .with(catalog::ServerCatalogMcpModule::prelude())?
        .with(local_projects.catalog_module())?
        .with(plan::PlanMcpModule::management(plan_deps.clone()))?
        .with(catalog::ServerCatalogMcpModule::remainder())?
        .with(plan::PlanMcpModule::remainder(plan_deps))?
        .with(signals::SignalMcpModule::new(signal_deps))?
        .with(spur_solver::mcp::SolverMcpModule::new(
            shared_solver_service(repo_root),
        ))?
        .with_alias("code_search", "code_symbol_search")?;
    let jev_enabled = std::env::var("SPUR_JEV_ENABLED").as_deref() == Ok("1")
        || repo_root
            .map(|repo_root| match spur_acp::config::load_layered(repo_root) {
                Ok(config) => config.jev.enabled,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        repo_root = %repo_root.display(),
                        "failed to load layered config for Jev registry gate; treating config as disabled"
                    );
                    false
                }
            })
            .unwrap_or(false);
    if jev_enabled {
        let catalog = spur_solver::rules::manifest_registry();
        let executable_rule_ids = spur_solver::rules::manifest_executable_rule_ids();
        let rules = catalog
            .rules()
            .iter()
            .filter(|rule| executable_rule_ids.iter().any(|id| id == rule.id()))
            .map(|rule| {
                let serialized = serde_json::to_value(rule).unwrap_or_default();
                let summary = serialized
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_else(|| rule.primitive())
                    .to_owned();
                spur_jev::snapshot::RuleCard {
                    rule_id: rule.id().to_owned(),
                    family: rule.family().to_owned(),
                    summary,
                }
            })
            .collect();
        let snapshot = spur_jev::snapshot::CatalogSnapshot {
            language_version: catalog.schema_version(),
            rules,
        };
        builder = builder.with(spur_jev::mcp::JevMcpModule::from_env(snapshot))?;
    }
    if let Some(context_service_client) = context_service_client(context_service_config) {
        builder = builder.with(context_service_client)?;
    }
    Ok(builder
        .with(skills_catalog::SkillsCatalogMcpModule::new(repo_root))?
        .build())
}

fn context_service_client(
    config: &ContextServiceConfig,
) -> Option<context_service::ContextServiceClient> {
    let base_url = std::env::var("SPUR_CONTEXT_SERVICE_URL")
        .ok()
        .and_then(non_empty_trimmed)
        .or_else(|| non_empty_trimmed(config.url.clone()))?;
    let bearer_token = std::env::var("SPUR_CONTEXT_SERVICE_TOKEN")
        .ok()
        .and_then(non_empty_trimmed)
        .or_else(|| config.token.clone().and_then(non_empty_trimmed));
    match context_service::ContextServiceClient::with_optional_token(base_url, bearer_token) {
        Ok(client) => Some(client),
        Err(_error) => {
            tracing::warn!("rejected insecure authenticated context-service endpoint");
            None
        }
    }
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub fn catalog_tool_registry() -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let local_projects = LocalProjectMcpComposition::from_environment();
    let builder = spur_mcp::ToolRegistry::builder()
        .with(delegation::DelegationMcpModule::new(
            delegation::DelegationMcpDeps::catalog_only(),
        ))?
        .with(catalog::ServerCatalogMcpModule::prelude())?
        .with(local_projects.catalog_module())?
        .with(plan::PlanMcpModule::management(
            plan::PlanMcpDeps::catalog_only(),
        ))?
        .with(catalog::ServerCatalogMcpModule::remainder())?
        .with(plan::PlanMcpModule::remainder(
            plan::PlanMcpDeps::catalog_only(),
        ))?
        .with(signals::SignalMcpModule::new(signals::SignalMcpDeps {
            pm_service: None,
            event_sink: None,
            feature_gate: crate::server::community_feature_gate(),
        }))?
        .with(spur_solver::mcp::SolverMcpModule::catalog_only())?
        .with_alias("code_search", "code_symbol_search")?;
    Ok(builder
        .with(skills_catalog::SkillsCatalogMcpModule::new(None))?
        .build())
}

pub fn tools_list() -> Vec<spur_mcp::ToolDefinition> {
    catalog_tool_registry()
        .expect("core MCP tool registry must be valid")
        .list_tools()
}

pub fn worker_tool_registry() -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let context_service_config = ContextServiceConfig {
        url: String::new(),
        ..ContextServiceConfig::default()
    };
    worker_tool_registry_with_context_service(&context_service_config)
}

pub fn worker_tool_registry_with_context_service(
    context_service_config: &ContextServiceConfig,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    worker_tool_registry_with_client(context_service_client(context_service_config))
}

fn worker_tool_registry_with_client(
    context_service_client: Option<context_service::ContextServiceClient>,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    worker_tool_registry_with_client_and_repo_root(context_service_client, None)
}

fn worker_tool_registry_with_client_and_repo_root(
    context_service_client: Option<context_service::ContextServiceClient>,
    repo_root: Option<&Path>,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    worker_tool_registry_with_families_inner(
        context_service_client,
        repo_root,
        &McpFamily::ALL.iter().copied().collect(),
    )
}

fn worker_tool_registry_with_families_inner(
    context_service_client: Option<context_service::ContextServiceClient>,
    repo_root: Option<&Path>,
    families: &std::collections::BTreeSet<McpFamily>,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    let mut builder = spur_mcp::ToolRegistry::builder();
    if families.contains(&McpFamily::Pm) {
        builder = builder.with(catalog::WorkerCatalogMcpModule::prelude())?;
    }
    if families.contains(&McpFamily::Worker) {
        builder = builder.with(worker::WorkerReadMcpModule::plan(
            worker::WorkerReadMcpDeps::catalog_only(),
        ))?;
        builder = builder.with(crate::worker_notebook::NotebookReadCatalog)?;
    }
    match (
        families.contains(&McpFamily::Graph),
        families.contains(&McpFamily::Analyst),
    ) {
        (true, true) => {
            builder = builder.with(catalog::WorkerCatalogMcpModule::remainder())?;
        }
        (true, false) => {
            builder = builder.with(catalog::WorkerCatalogMcpModule::graph())?;
        }
        (false, true) => {
            builder = builder.with(catalog::WorkerCatalogMcpModule::analyst())?;
        }
        (false, false) => {}
    }
    if families.contains(&McpFamily::Worker) {
        builder = builder.with(worker::WorkerReadMcpModule::artifact(
            worker::WorkerReadMcpDeps::catalog_only(),
        ))?;
    }
    if families.contains(&McpFamily::Signals) {
        builder = builder.with(signals::SignalMcpModule::new(signals::SignalMcpDeps {
            pm_service: None,
            event_sink: None,
            feature_gate: crate::server::community_feature_gate(),
        }))?;
    }
    if families.contains(&McpFamily::Review) {
        builder = builder.with(review_verdict::ReviewVerdictMcpModule)?;
    }
    if families.contains(&McpFamily::Solver) {
        builder = builder.with(spur_solver::mcp::SolverMcpModule::new(
            shared_solver_service(repo_root),
        ))?;
    }
    if families.contains(&McpFamily::Graph) {
        builder = builder.with_alias("code_search", "code_symbol_search")?;
    }
    if families.contains(&McpFamily::Context) {
        if let Some(context_service_client) = context_service_client {
            builder = builder.with(context_service_client)?;
        }
    }
    if families.contains(&McpFamily::Skills) {
        builder = builder.with(skills_catalog::SkillsCatalogMcpModule::new(repo_root))?;
    }
    Ok(builder
        .with_denied_tool_calls(WORKER_DENIED_TOOL_CALLS.iter().copied())
        .build())
}

struct SharedSolverServices {
    unrooted: Arc<spur_solver::service::SolverService>,
    rooted: Mutex<HashMap<PathBuf, Weak<spur_solver::service::SolverService>>>,
}

fn shared_solver_service(repo_root: Option<&Path>) -> Arc<spur_solver::service::SolverService> {
    static SERVICES: OnceLock<SharedSolverServices> = OnceLock::new();
    let services = SERVICES.get_or_init(|| SharedSolverServices {
        unrooted: Arc::new(spur_solver::service::SolverService::new()),
        rooted: Mutex::new(HashMap::new()),
    });
    let Some(repo_root) = repo_root else {
        return Arc::clone(&services.unrooted);
    };

    let mut rooted = services
        .rooted
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(service) = rooted.get(repo_root).and_then(Weak::upgrade) {
        return service;
    }

    let service = Arc::new(services.unrooted.as_ref().clone().with_repo_root(repo_root));
    rooted.insert(repo_root.to_path_buf(), Arc::downgrade(&service));
    service
}

pub(crate) fn worker_tool_dispatch_with_families(
    context_service_config: &ContextServiceConfig,
    repo_root: Option<&Path>,
    families: &std::collections::BTreeSet<McpFamily>,
) -> (
    Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError>,
    Option<context_service::ContextServiceClient>,
) {
    let context_service_client = context_service_client(context_service_config);
    let registry = worker_tool_registry_with_families_inner(
        context_service_client.clone(),
        repo_root,
        families,
    );
    (registry, context_service_client)
}

pub(crate) fn is_context_service_tool_name(name: &str) -> bool {
    context_service::is_tool_name(name)
}

pub fn worker_tools_list() -> Vec<spur_mcp::ToolDefinition> {
    worker_tool_registry()
        .expect("core worker MCP tool registry must be valid")
        .list_tools()
}

/// Compose the worker catalog from an already-resolved leaf set.
pub fn worker_tool_registry_for_families(
    families: &std::collections::BTreeSet<McpFamily>,
) -> Result<spur_mcp::ToolRegistry, spur_mcp::ToolRegistryError> {
    worker_tool_registry_with_families_inner(None, None, families)
}

/// Claude-format tool names (`mcp__spur-worker-mcp__<tool>`) for every tool
/// the curated worker MCP server advertises via `list_tools`. Used to augment
/// Claude agent-profile `tools:` allowlists so a restricted persona keeps the
/// worker MCP surface its delegation ships.
pub(crate) fn worker_mcp_claude_tool_names() -> Vec<String> {
    let registry = worker_tool_registry().expect("core worker MCP tool registry must be valid");
    worker_mcp_claude_tool_names_for_registry(&registry)
}

pub(crate) fn worker_mcp_claude_tool_names_for_registry(
    registry: &spur_mcp::ToolRegistry,
) -> Vec<String> {
    registry
        .list_tools()
        .into_iter()
        .map(|def| {
            format!(
                "mcp__{}__{}",
                crate::worker_server::WORKER_MCP_SERVER_NAME,
                def.name
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use spur_acp::config::{ContextServiceAuthMode, ContextServiceConfig};

    fn config(mode: ContextServiceAuthMode) -> ContextServiceConfig {
        ContextServiceConfig {
            url: "https://context.example.test".to_owned(),
            auth_mode: mode,
            profile: "profile-a".to_owned(),
            public_id_hint: None,
            token: None,
        }
    }

    #[test]
    fn env_api_key_forces_api_key_auth_even_in_legacy_mode() {
        let auth = resolve_context_auth(
            &config(ContextServiceAuthMode::None),
            Some("spur_live_x_secret".to_owned()),
            None,
            None,
        )
        .expect("environment API key must force API-key auth");
        assert!(matches!(auth, ContextServiceAuth::ApiKey(_)));
    }

    #[test]
    fn api_key_mode_uses_stored_key() {
        let auth = resolve_context_auth(
            &config(ContextServiceAuthMode::ApiKey),
            None,
            None,
            Some(SecretString::from("spur_live_stored_secret")),
        )
        .expect("stored API key must produce API-key auth");
        assert!(matches!(auth, ContextServiceAuth::ApiKey(_)));
    }

    #[test]
    fn api_key_mode_without_any_key_skips_client_registration() {
        assert!(
            resolve_context_auth(
                &config(ContextServiceAuthMode::ApiKey),
                None,
                Some("legacy-bearer".to_owned()),
                None,
            )
            .is_none(),
            "explicit API-key mode without a resolvable key must not silently downgrade to bearer or anonymous auth"
        );
    }

    #[test]
    fn legacy_mode_maps_token_to_oauth_bearer() {
        let auth = resolve_context_auth(
            &config(ContextServiceAuthMode::None),
            None,
            Some("legacy-bearer".to_owned()),
            None,
        )
        .expect("legacy token keeps bearer auth");
        assert!(matches!(auth, ContextServiceAuth::OAuthBearer(_)));
    }

    #[test]
    fn legacy_mode_without_credentials_stays_anonymous() {
        let auth = resolve_context_auth(&config(ContextServiceAuthMode::None), None, None, None)
            .expect("anonymous mode keeps registering the unauthenticated client");
        assert!(matches!(auth, ContextServiceAuth::None));
    }
    use rmcp::model::ErrorCode;
    use serde_json::{json, Value};
    use spur_mcp::{ServerKind, ToolAuthority, ToolCallContext};

    fn snapshot_files(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            current: &Path,
            snapshot: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
        ) {
            for entry in std::fs::read_dir(current).expect("read snapshot directory") {
                let entry = entry.expect("snapshot entry");
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, snapshot);
                } else {
                    let relative = path.strip_prefix(root).expect("relative snapshot path");
                    snapshot.insert(
                        relative.to_path_buf(),
                        std::fs::read(&path).expect("read snapshot file"),
                    );
                }
            }
        }

        let mut snapshot = std::collections::BTreeMap::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    fn expected_skill_search_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "minLength": 1 },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "default": 5
                },
                "source": { "type": "string" }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn expected_skill_read_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string", "minLength": 1 },
                "resource": { "type": "string" }
            },
            "required": ["skill_id"],
            "additionalProperties": false
        })
    }

    fn expected_skill_navigate_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Full-text query over skill PageIndex nodes. Required when root is omitted."
                },
                "root": {
                    "type": "string",
                    "description": "Skill id or skill_id:node_id. When set, expand one tree hop instead of FTS."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "default": 5
                },
                "source": { "type": "string" },
                "include_lede": {
                    "type": "boolean",
                    "default": true,
                    "description": "When true, include node lede snippets. When false, omit lede fields."
                }
            },
            "additionalProperties": false
        })
    }

    fn tool_result_json(response: spur_mcp::response::JsonRpcResponse) -> serde_json::Value {
        assert!(
            response.error.is_none(),
            "unexpected error: {:?}",
            response.error
        );
        let text = response
            .result
            .as_ref()
            .and_then(|result| result["content"][0]["text"].as_str())
            .expect("MCP tool result text");
        serde_json::from_str(text).expect("JSON tool result")
    }

    #[test]
    fn skill_catalog_schemas_are_identical_in_unrooted_catalogs() {
        for (catalog_name, definitions) in
            [("brain", tools_list()), ("worker", worker_tools_list())]
        {
            let search = definitions
                .iter()
                .find(|definition| definition.name == "skill_search")
                .unwrap_or_else(|| panic!("{catalog_name} catalog missing skill_search"));
            let read = definitions
                .iter()
                .find(|definition| definition.name == "skill_read")
                .unwrap_or_else(|| panic!("{catalog_name} catalog missing skill_read"));
            let navigate = definitions
                .iter()
                .find(|definition| definition.name == "skill_navigate")
                .unwrap_or_else(|| panic!("{catalog_name} catalog missing skill_navigate"));

            assert_eq!(search.input_schema, expected_skill_search_schema());
            assert_eq!(read.input_schema, expected_skill_read_schema());
            assert_eq!(navigate.input_schema, expected_skill_navigate_schema());
            assert_eq!(
                definitions
                    .iter()
                    .rev()
                    .take(3)
                    .map(|definition| definition.name.as_str())
                    .collect::<Vec<_>>(),
                ["skill_navigate", "skill_read", "skill_search"],
                "{catalog_name} must append skills without reordering existing tools"
            );
        }
    }

    #[tokio::test]
    async fn unrooted_skill_catalog_call_requires_repository_authority() {
        let registry = worker_tool_registry().expect("worker registry");
        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);

        let error = match registry
            .call_tool(context, "skill_search", json!({ "query": "verification" }))
            .await
        {
            Ok(_) => panic!("unrooted skill search must fail"),
            Err(error) => error,
        };

        assert_eq!(error.code, ErrorCode(-32001));
        assert!(error.message.contains("repository authority root"));
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "authority_root_required",
                "write_effect": "none"
            }))
        );
    }

    #[tokio::test]
    async fn unrooted_skill_catalog_rejects_before_parsing_arguments() {
        let registry = worker_tool_registry().expect("worker registry");
        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);

        let error = match registry.call_tool(context, "skill_search", json!({})).await {
            Ok(_) => panic!("unrooted skill search must fail"),
            Err(error) => error,
        };

        assert_eq!(error.code, ErrorCode(-32001));
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "authority_root_required",
                "write_effect": "none"
            }))
        );
    }

    #[tokio::test]
    async fn rooted_worker_skill_read_rejects_malformed_arguments_as_invalid_params() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let before = snapshot_files(temp.path());
        let registry = worker_tool_registry_with_client_and_repo_root(None, Some(temp.path()))
            .expect("rooted worker registry");

        for (case, args) in [
            ("missing skill_id", json!({})),
            ("wrong skill_id type", json!({ "skill_id": 7 })),
            (
                "unknown field",
                json!({ "skill_id": "opaque-reference", "unexpected": true }),
            ),
        ] {
            let context =
                ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
            let response = registry.call_json_tool(context, "skill_read", args).await;
            let error = response.error.unwrap_or_else(|| panic!("{case} must fail"));

            assert_eq!(error.code, -32602, "unexpected code for {case}");
            assert_eq!(
                error.data,
                Some(json!({
                    "error_kind": "invalid_query",
                    "write_effect": "none"
                })),
                "unexpected error data for {case}"
            );
        }

        assert_eq!(snapshot_files(temp.path()), before);
    }

    #[tokio::test]
    async fn rooted_worker_skill_read_rejects_empty_id_as_invalid_params() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let before = snapshot_files(temp.path());
        let registry = worker_tool_registry_with_client_and_repo_root(None, Some(temp.path()))
            .expect("rooted worker registry");
        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);

        let response = registry
            .call_json_tool(context, "skill_read", json!({ "skill_id": "" }))
            .await;
        let error = response.error.expect("empty skill_id must fail");

        assert_eq!(error.code, -32602);
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "invalid_query",
                "write_effect": "none"
            }))
        );
        assert_eq!(snapshot_files(temp.path()), before);
    }

    #[tokio::test]
    async fn rooted_worker_skill_catalog_searches_metadata_and_reads_exact_text() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let skill_dir = temp.path().join("assets/skills/catalog-needle");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        let document = concat!(
            "---\n",
            "name: catalog-needle\n",
            "description: Find a unique catalog needle for MCP tests\n",
            "role: worker\n",
            "---\n\n",
            "# Catalog Needle\n\n",
            "EXACT_INSTRUCTION_BODY_MUST_NOT_LEAK_FROM_SEARCH\n",
        );
        std::fs::write(skill_dir.join("SKILL.md"), document).expect("write skill");
        for index in 0..6 {
            let candidate_dir = temp
                .path()
                .join(format!("assets/skills/common-candidate-{index}"));
            std::fs::create_dir_all(&candidate_dir).expect("create candidate dir");
            std::fs::write(
                candidate_dir.join("SKILL.md"),
                format!(
                    "---\nname: common-candidate-{index}\ndescription: Shared bounded catalog candidate\nrole: worker\n---\n\n# Candidate {index}\n"
                ),
            )
            .expect("write candidate skill");
        }
        let before = snapshot_files(temp.path());

        let brain = brain_tool_registry_for_repo_root(
            delegation::DelegationMcpDeps::catalog_only(),
            plan::PlanMcpDeps::catalog_only(),
            signals::SignalMcpDeps {
                pm_service: None,
                event_sink: None,
                feature_gate: crate::server::community_feature_gate(),
            },
            &ContextServiceConfig::default(),
            temp.path(),
        )
        .expect("rooted brain registry");
        for tool_name in ["skill_search", "skill_read", "skill_navigate"] {
            assert!(
                brain
                    .list_tools()
                    .iter()
                    .any(|definition| definition.name == tool_name),
                "rooted brain registry missing {tool_name}"
            );
        }
        let registry = worker_tool_registry_with_client_and_repo_root(None, Some(temp.path()))
            .expect("rooted worker registry");

        let request_id = json!(41);
        let context = ToolCallContext::new(
            ServerKind::Worker,
            ToolAuthority::Worker,
            None,
            Some(&request_id),
        );
        let search = registry
            .call_json_tool(
                context,
                "skill_search",
                json!({ "query": "unique catalog needle" }),
            )
            .await;
        let search = tool_result_json(search);
        assert!(search["results"]
            .as_array()
            .is_some_and(|results| (1..=5).contains(&results.len())));
        assert_eq!(search["results"][0]["name"], "catalog-needle");
        assert!(search.get("content").is_none());
        assert!(!search
            .to_string()
            .contains("EXACT_INSTRUCTION_BODY_MUST_NOT_LEAK_FROM_SEARCH"));
        assert_eq!(search["results"][0]["source"], "bundled");

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let bounded = registry
            .call_json_tool(
                context,
                "skill_search",
                json!({ "query": "shared bounded catalog candidate" }),
            )
            .await;
        let bounded = tool_result_json(bounded);
        assert_eq!(bounded["results"].as_array().map(Vec::len), Some(5));

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let exact_source = registry
            .call_json_tool(
                context,
                "skill_search",
                json!({
                    "query": "shared bounded catalog candidate",
                    "source": "Bundled"
                }),
            )
            .await;
        let exact_source = tool_result_json(exact_source);
        assert_eq!(exact_source["results"].as_array().map(Vec::len), Some(0));

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let over_limit = registry
            .call_json_tool(
                context,
                "skill_search",
                json!({ "query": "candidate", "limit": 6 }),
            )
            .await;
        let error = over_limit.error.expect("over-limit search error");
        assert_eq!(error.code, -32602);
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "invalid_query",
                "write_effect": "none"
            }))
        );

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let null_limit = registry
            .call_json_tool(
                context,
                "skill_search",
                json!({ "query": "candidate", "limit": null }),
            )
            .await;
        let error = null_limit.error.expect("null limit search error");
        assert_eq!(error.code, -32602);
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "invalid_query",
                "write_effect": "none"
            }))
        );

        let skill_id = search["results"][0]["skill_id"]
            .as_str()
            .expect("opaque skill id");
        let request_id = json!(42);
        let context = ToolCallContext::new(
            ServerKind::Worker,
            ToolAuthority::Worker,
            None,
            Some(&request_id),
        );
        let read = registry
            .call_json_tool(context, "skill_read", json!({ "skill_id": skill_id }))
            .await;
        let read = tool_result_json(read);
        assert_eq!(read["resource"], "SKILL.md");
        assert_eq!(read["media_type"], "text/markdown");
        assert_eq!(read["content"], document);

        let request_id = json!(43);
        let context = ToolCallContext::new(
            ServerKind::Worker,
            ToolAuthority::Worker,
            None,
            Some(&request_id),
        );
        let navigate = registry
            .call_json_tool(
                context,
                "skill_navigate",
                json!({ "query": "unique catalog needle" }),
            )
            .await;
        let navigate = tool_result_json(navigate);
        assert!(navigate["catalog_revision"].as_str().is_some());
        assert!(navigate["hits"]
            .as_array()
            .is_some_and(|hits| (1..=5).contains(&hits.len())));
        assert!(navigate["hits"][0].get("lede").is_some());
        assert!(navigate.get("content").is_none());

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let navigate_no_lede = registry
            .call_json_tool(
                context,
                "skill_navigate",
                json!({
                    "query": "unique catalog needle",
                    "include_lede": false
                }),
            )
            .await;
        let navigate_no_lede = tool_result_json(navigate_no_lede);
        assert!(navigate_no_lede["hits"]
            .as_array()
            .is_some_and(|hits| !hits.is_empty()));
        assert!(navigate_no_lede["hits"][0].get("lede").is_none());

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let navigate_root = registry
            .call_json_tool(
                context,
                "skill_navigate",
                json!({ "root": skill_id, "limit": 5 }),
            )
            .await;
        let navigate_root = tool_result_json(navigate_root);
        assert!(navigate_root["hits"]
            .as_array()
            .is_some_and(|hits| !hits.is_empty()));

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let missing_query = registry
            .call_json_tool(context, "skill_navigate", json!({}))
            .await;
        let error = missing_query
            .error
            .expect("skill_navigate without query/root must fail");
        assert_eq!(error.code, -32602);
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "invalid_query",
                "write_effect": "none"
            }))
        );

        let context = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let unknown_field = registry
            .call_json_tool(
                context,
                "skill_navigate",
                json!({ "query": "needle", "unexpected": true }),
            )
            .await;
        let error = unknown_field
            .error
            .expect("skill_navigate unknown field must fail");
        assert_eq!(error.code, -32602);
        assert_eq!(
            error.data,
            Some(json!({
                "error_kind": "invalid_query",
                "write_effect": "none"
            }))
        );

        assert_eq!(snapshot_files(temp.path()), before);
    }

    #[tokio::test]
    async fn worker_registry_dispatches_allowed_read_tools_through_core_module() {
        let registry = worker_tool_registry().expect("worker registry");

        for (tool_name, args) in [
            ("get_plan_status", json!({})),
            ("get_task_diff", json!({})),
            ("fetch_outcome_artifact", json!({})),
        ] {
            let ctx = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
            let err = match registry.call_tool(ctx, tool_name, args).await {
                Ok(_) => panic!("{tool_name} should reject missing required arguments"),
                Err(err) => err,
            };

            assert_eq!(
                err.code,
                ErrorCode(-32602),
                "{tool_name} should reach its real worker read handler"
            );
            assert!(
                !err.message.contains("must be dispatched"),
                "{tool_name} must not be a catalog-only placeholder: {}",
                err.message
            );
        }
    }

    #[test]
    fn worker_mcp_claude_tool_names_cover_the_advertised_registry() {
        let names = worker_mcp_claude_tool_names();
        let advertised = worker_tools_list();
        assert_eq!(names.len(), advertised.len());
        for def in &advertised {
            let expected = format!("mcp__spur-worker-mcp__{}", def.name);
            assert!(names.contains(&expected), "missing {expected}");
        }
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "names must not repeat");

        // Signals, graph, and analyst tools must be present…
        for sentinel in [
            "mcp__spur-worker-mcp__report_progress",
            "mcp__spur-worker-mcp__report_signal",
            "mcp__spur-worker-mcp__code_read_symbol",
            "mcp__spur-worker-mcp__query",
            "mcp__spur-worker-mcp__solve_rule_spec",
            "mcp__spur-worker-mcp__solve_rules",
            "mcp__spur-worker-mcp__solve_constraint_spec",
            "mcp__spur-worker-mcp__solve_constraint_check",
            "mcp__spur-worker-mcp__solve_constraints",
            "mcp__spur-worker-mcp__solve_smt",
            "mcp__spur-worker-mcp__get_solve_result",
        ] {
            assert!(
                names.iter().any(|name| name == sentinel),
                "missing sentinel {sentinel}"
            );
        }
        // …and brain-only tools must not leak in.
        assert!(!names.iter().any(|name| name.contains("delegate_to_worker")));
    }

    #[test]
    fn worker_tool_schemas_meet_bedrock_shape_requirements() {
        fn find_forbidden_keyword(value: &Value, path: &str) -> Option<String> {
            match value {
                Value::Object(map) => {
                    for keyword in ["oneOf", "allOf"] {
                        if map.contains_key(keyword) {
                            return Some(format!("{path}.{keyword}"));
                        }
                    }
                    if let Some(type_value) = map.get("type") {
                        if type_value.is_array() {
                            return Some(format!("{path}.type (union array)"));
                        }
                    }
                    map.iter().find_map(|(key, value)| {
                        find_forbidden_keyword(value, &format!("{path}.{key}"))
                    })
                }
                Value::Array(values) => values.iter().enumerate().find_map(|(index, value)| {
                    find_forbidden_keyword(value, &format!("{path}[{index}]"))
                }),
                _ => None,
            }
        }

        let registry = worker_tool_registry().expect("worker registry");
        let offenders = registry
            .list_tools()
            .into_iter()
            .filter_map(|tool| {
                if tool.input_schema["type"] != "object" {
                    return Some(format!("{} has a non-object root", tool.name));
                }
                find_forbidden_keyword(&tool.input_schema, "$")
                    .map(|path| format!("{} at {path}", tool.name))
            })
            .collect::<Vec<_>>();

        assert!(
            offenders.is_empty(),
            "Kiro Bedrock requires object roots and rejects oneOf/allOf/type-unions in worker tool schemas: {}",
            offenders.join(", ")
        );
    }

    #[tokio::test]
    async fn worker_registry_dispatches_solver_tools() {
        let registry = worker_tool_registry().expect("worker registry");

        let guide = registry
            .call_tool(
                ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None),
                "solve_rule_spec",
                json!({}),
            )
            .await;
        assert!(guide.is_ok(), "rule guide must dispatch without Z3");

        let language = registry
            .call_tool(
                ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None),
                "solve_constraint_spec",
                json!({}),
            )
            .await;
        assert!(language.is_ok(), "language guide must dispatch without Z3");

        for tool_name in [
            "solve_rules",
            "solve_constraint_check",
            "solve_constraints",
            "solve_smt",
            "get_solve_result",
        ] {
            let context =
                ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
            let error = match registry.call_tool(context, tool_name, json!({})).await {
                Ok(_) => panic!("missing solver arguments must be rejected: {tool_name}"),
                Err(error) => error,
            };

            assert_eq!(
                error.code,
                ErrorCode(-32602),
                "{tool_name} must reach the live solver module"
            );
        }
    }

    #[test]
    fn configured_worker_mcp_claude_tool_names_include_external_tools() {
        let registry = worker_tool_registry_with_client(Some(
            ContextServiceClient::with_optional_token("http://127.0.0.1:9/context", None)
                .expect("loopback context-service client"),
        ))
        .expect("configured worker registry");

        let names = worker_mcp_claude_tool_names_for_registry(&registry);

        assert!(names.contains(&"mcp__spur-worker-mcp__external_code_read".to_owned()));
        assert!(names.contains(&"mcp__spur-worker-mcp__external_index".to_owned()));
    }

    #[tokio::test]
    async fn worker_registry_denies_brain_only_plan_tools_with_authorization_error() {
        let registry = worker_tool_registry().expect("worker registry");
        let listed: Vec<String> = registry
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        for tool_name in ["submit_plan", "review_task", "submit_plan_mutation"] {
            assert!(
                !listed.iter().any(|name| name == tool_name),
                "{tool_name} must not be advertised to workers"
            );

            let ctx = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
            let err = match registry.call_tool(ctx, tool_name, json!({})).await {
                Ok(_) => panic!("brain-only worker call must be rejected: {tool_name}"),
                Err(err) => err,
            };
            assert_eq!(
                err.code,
                ErrorCode(-32001),
                "{tool_name} should fail authorization, not tool lookup"
            );
            assert!(
                err.message.contains("not authorized"),
                "{tool_name} denial should explain authorization: {}",
                err.message
            );
        }
    }

    fn listed_names(
        families: &std::collections::BTreeSet<McpFamily>,
    ) -> std::collections::BTreeSet<String> {
        worker_tool_registry_for_families(families)
            .expect("family-filtered worker registry")
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    #[test]
    fn solver_only_registry_omits_other_leaves() {
        let families = resolve_mcp_families(true, &["solver"], &[] as &[&str])
            .unwrap()
            .unwrap();
        let names = listed_names(&families);
        assert!(names.contains("solve_rule_spec"));
        assert!(names.contains("solve_constraints"));
        assert!(!names.contains("code_read_symbol"));
        assert!(!names.contains("query"));
        assert!(!names.contains("get_issue"));
        assert!(!names.contains("skill_navigate"));
        assert!(!names.contains("report_signal"));
        assert!(!names.contains("get_plan_status"));
        assert!(!names.contains("submit_review_verdict"));
        assert!(!names.contains("delegate_to_worker"));
        assert!(!names.contains("code_search"));
    }

    #[test]
    fn core_minus_pm_registry_keeps_solver_and_analyst() {
        let families = resolve_mcp_families(true, &["core"], &["pm"])
            .unwrap()
            .unwrap();
        let names = listed_names(&families);
        assert!(names.contains("solve_rule_spec"));
        assert!(names.contains("query"));
        assert!(names.contains("skill_navigate"));
        assert!(!names.contains("get_issue"));
        assert!(!names.contains("list_issues"));
        assert!(!names.contains("code_read_symbol"));
        assert!(!names.contains("create_issue"));
    }

    #[tokio::test]
    async fn omitted_family_tool_is_unknown() {
        let families = resolve_mcp_families(true, &["solver"], &[] as &[&str])
            .unwrap()
            .unwrap();
        let registry = worker_tool_registry_for_families(&families).unwrap();
        let ctx = ToolCallContext::new(ServerKind::Worker, ToolAuthority::Worker, None, None);
        let err = match registry
            .call_tool(ctx, "code_read_symbol", json!({"selector": "x"}))
            .await
        {
            Ok(_) => panic!("omitted graph tool must not dispatch"),
            Err(err) => err,
        };
        assert_eq!(err.code, ErrorCode(-32601));
    }
}
