pub mod entries;
pub mod hooks;
pub mod layered;
pub mod validator;

pub use entries::{
    CommandsConfig, DisplayConfig, IngestBinding, PermissionsConfig, ResponseBinding,
    StaticCommandDecl,
};
pub use hooks::{
    ArgsTemplateKind, DispatchKind, IngestParserKind, ItemSchemaKind, ResponseRenderKind,
};
pub use layered::{load_layered, merge_tables};
pub use validator::{validate_agent_config, ConfigError};

use crate::domain::delegation::TimeoutFallback;
use crate::profile_strategy::ProfileConfig;
use crate::types::{AgentKind, AgentRole, CostTier, TransportKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Per-agent task-routing descriptor. Feeds both the brain prompt and
/// `list_available_workers` tool response. See design spec section A.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DelegationDescriptor {
    /// One-line human summary. Rendered into the workers-block of the
    /// brain prompt when non-empty.
    pub description: Option<String>,
    /// Routing preference signal. Specialists are preferred when their
    /// good_for matches; generalists are fallback.
    pub tier: Option<Tier>,
    /// Positive task patterns. Used by the brain to route.
    pub good_for: Vec<String>,
    /// Negative task patterns. Soft signal; brain MAY override with
    /// stated rationale when no better agent exists.
    pub avoid_for: Vec<String>,
    /// Held back from workers-block; injected into per-dispatch task
    /// prompt only.
    pub strengths: Vec<String>,
    /// Held back from workers-block; injected into per-dispatch task
    /// prompt only.
    pub limitations: Vec<String>,
    /// Held back from routing; shown to brain when dispatching so it
    /// can shape CONTEXT appropriately.
    pub input_expectations: Option<String>,
    /// Routing-relevant via `list_available_workers`. Brain uses for
    /// EXPECTED_OUTPUT section of dispatched task prompt.
    pub output_shape: Option<String>,
    /// Default true. When false, user fields are used verbatim
    /// (including empty vecs — no built-in merge).
    // Field-level default needed even with struct-level `#[serde(default)]`:
    // the struct-level default only fires when the whole block is absent.
    #[serde(default = "default_true")]
    pub inherit_defaults: bool,
}

#[cfg(test)]
mod mcp_servers_config_tests {
    use super::*;

    fn http_entry(name: &str) -> McpServerEntry {
        McpServerEntry {
            name: name.to_owned(),
            enabled: true,
            transport: McpServerTransport::Http {
                url: "https://mcp.example.com/sse".to_owned(),
                headers: std::collections::HashMap::new(),
            },
        }
    }

    fn stdio_entry(name: &str) -> McpServerEntry {
        McpServerEntry {
            name: name.to_owned(),
            enabled: true,
            transport: McpServerTransport::Stdio {
                command: "npx".to_owned(),
                args: vec![
                    "-y".to_owned(),
                    "@modelcontextprotocol/server-github".to_owned(),
                ],
                env: std::collections::HashMap::new(),
            },
        }
    }

    #[test]
    fn toml_round_trip_preserves_entries() {
        let src = r#"
[[mcp_servers.entries]]
name = "github"
enabled = false

[mcp_servers.entries.transport]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[mcp_servers.entries.transport.env]
GITHUB_TOKEN = "tok"
"#;
        let cfg: SpurConfig = toml::from_str(src).expect("parse");
        assert_eq!(cfg.mcp_servers.entries.len(), 1);
        let entry = &cfg.mcp_servers.entries[0];
        assert_eq!(entry.name, "github");
        assert!(!entry.enabled);
        let out = toml::to_string(&cfg).expect("serialize");
        let back: SpurConfig = toml::from_str(&out).expect("re-parse");
        assert_eq!(back.mcp_servers, cfg.mcp_servers);
    }

    #[test]
    fn both_transport_blocks_is_deserialize_error() {
        let src = r#"
[[mcp_servers.entries]]
name = "bad"

[mcp_servers.entries.transport]
transport = "stdio"
command = "npx"
url = "https://x"
"#;
        assert!(toml::from_str::<SpurConfig>(src).is_err());
    }

    #[test]
    fn jute_debug_transport_round_trips_without_user_managed_process_fields() {
        let src = r#"
[[mcp_servers.entries]]
name = "jute-debug"
enabled = true

[mcp_servers.entries.transport]
transport = "jute_debug"
"#;
        let cfg: SpurConfig = toml::from_str(src).expect("parse jute debug transport");
        let entry = &cfg.mcp_servers.entries[0];
        assert!(matches!(entry.transport, McpServerTransport::JuteDebug));
        entry.validate().expect("managed transport is valid");

        let out = toml::to_string(&cfg).expect("serialize");
        assert!(out.contains("transport = \"jute_debug\""), "{out}");
        let back: SpurConfig = toml::from_str(&out).expect("re-parse");
        assert_eq!(back.mcp_servers, cfg.mcp_servers);
    }

    #[test]
    fn validate_rejects_reserved_and_empty() {
        let mut entry = http_entry("spur-mcp");
        assert!(entry.validate().is_err(), "reserved name");
        entry = http_entry("notebook");
        assert!(entry.validate().is_err());
        entry = http_entry("spur-worker-mcp");
        assert!(entry.validate().is_err());
        entry.name = "  ".to_owned();
        assert!(entry.validate().is_err(), "empty name");
    }

    #[test]
    fn validate_rejects_empty_payloads() {
        let mut entry = stdio_entry("s1");
        if let McpServerTransport::Stdio { command, .. } = &mut entry.transport {
            *command = "   ".to_owned();
        }
        assert!(entry.validate().is_err(), "empty command");

        let mut http = http_entry("h1");
        if let McpServerTransport::Http { url, .. } = &mut http.transport {
            *url = "ftp://nope".to_owned();
        }
        assert!(http.validate().is_err(), "non-http scheme");
        if let McpServerTransport::Http { url, .. } = &mut http.transport {
            *url = String::new();
        }
        assert!(http.validate().is_err(), "empty url");
    }

    #[test]
    fn upsert_replaces_in_place_and_remove_missing_errors() {
        let mut cfg = SpurConfig::default();
        ConfigPatch::McpServerUpsert {
            entry: http_entry("a"),
        }
        .apply(&mut cfg)
        .unwrap();
        ConfigPatch::McpServerUpsert {
            entry: stdio_entry("b"),
        }
        .apply(&mut cfg)
        .unwrap();
        let replaced = stdio_entry("a");
        ConfigPatch::McpServerUpsert { entry: replaced }
            .apply(&mut cfg)
            .unwrap();
        assert_eq!(
            cfg.mcp_servers.entries.len(),
            2,
            "upsert replaces, never duplicates"
        );
        assert_eq!(cfg.mcp_servers.entries[0].name, "a");
        assert!(matches!(
            cfg.mcp_servers.entries[0].transport,
            McpServerTransport::Stdio { .. }
        ));

        let err = ConfigPatch::McpServerRemove { name: "zzz".into() }.apply(&mut cfg);
        assert!(err.is_err(), "remove missing name must fail");
        ConfigPatch::McpServerRemove { name: "b".into() }
            .apply(&mut cfg)
            .unwrap();
        assert_eq!(cfg.mcp_servers.entries.len(), 1);
    }

    #[test]
    fn patch_section_id_is_mcp() {
        assert_eq!(
            ConfigPatch::McpServerUpsert {
                entry: http_entry("x")
            }
            .section_id(),
            "mcp"
        );
        assert_eq!(
            ConfigPatch::McpServerRemove { name: "x".into() }.section_id(),
            "mcp"
        );
    }
}

#[cfg(test)]
mod builtin_mcp_overrides_tests {
    use super::*;

    #[test]
    fn builtin_overrides_default_all_true() {
        let cfg = SpurConfig::default();
        let overrides = &cfg.mcp_servers.builtin_overrides;

        assert!(overrides.spur_mcp_enabled);
        assert!(overrides.notebook_enabled);
        assert!(overrides.worker_mcp_enabled);
    }

    #[test]
    fn builtin_overrides_toml_round_trip_preserves_false_override() {
        let src = r#"
[mcp_servers.builtin_overrides]
notebook_enabled = false
"#;
        let cfg: SpurConfig = toml::from_str(src).expect("parse builtin overrides");
        let overrides = &cfg.mcp_servers.builtin_overrides;
        assert!(overrides.spur_mcp_enabled);
        assert!(!overrides.notebook_enabled);
        assert!(overrides.worker_mcp_enabled);

        let out = toml::to_string(&cfg).expect("serialize builtin overrides");
        let back: SpurConfig = toml::from_str(&out).expect("re-parse builtin overrides");
        assert_eq!(back.mcp_servers, cfg.mcp_servers);
    }

    #[test]
    fn all_true_overrides_serialize_to_nothing() {
        let cfg = SpurConfig::default();
        let serialized = toml::to_string(&cfg).expect("serialize default config");

        assert!(!serialized.contains("builtin_overrides"));
    }

    #[test]
    fn builtin_toggle_applies_per_server() {
        for server in [
            BuiltinMcpServer::SpurMcp,
            BuiltinMcpServer::Notebook,
            BuiltinMcpServer::SpurWorkerMcp,
        ] {
            let mut cfg = SpurConfig::default();
            let patch = ConfigPatch::BuiltinMcpToggle {
                server,
                enabled: false,
            };

            assert_eq!(patch.section_id(), "mcp");
            patch.apply(&mut cfg).expect("apply builtin MCP toggle");

            let overrides = &cfg.mcp_servers.builtin_overrides;
            match server {
                BuiltinMcpServer::SpurMcp => {
                    assert!(!overrides.spur_mcp_enabled);
                    assert!(overrides.notebook_enabled);
                    assert!(overrides.worker_mcp_enabled);
                }
                BuiltinMcpServer::Notebook => {
                    assert!(overrides.spur_mcp_enabled);
                    assert!(!overrides.notebook_enabled);
                    assert!(overrides.worker_mcp_enabled);
                }
                BuiltinMcpServer::SpurWorkerMcp => {
                    assert!(overrides.spur_mcp_enabled);
                    assert!(overrides.notebook_enabled);
                    assert!(!overrides.worker_mcp_enabled);
                }
            }
        }
    }
}

impl Default for DelegationDescriptor {
    fn default() -> Self {
        Self {
            description: None,
            tier: None,
            good_for: Vec::new(),
            avoid_for: Vec::new(),
            strengths: Vec::new(),
            limitations: Vec::new(),
            input_expectations: None,
            output_shape: None,
            inherit_defaults: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Specialist,
    Generalist,
}

/// Configuration for a single registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique name for this agent (e.g., "kiro", "claude-code").
    pub name: String,
    /// Command to execute (e.g., "kiro-cli", "claude").
    pub command: String,
    /// Arguments to pass to the command (e.g., ["acp"], ["--experimental-acp"]).
    #[serde(default)]
    pub args: Vec<String>,
    /// Additional absolute workspace roots to expose to ACP sessions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,
    /// Which transport protocol to use.
    pub transport: TransportKind,
    /// Wire-level idiom used by the adapter layer for TUI rendering.
    /// Orthogonal to `transport`: multiple kinds share the same transport.
    /// Defaults to `Generic` for unknown agents (safe heuristic fallback).
    #[serde(default)]
    pub kind: AgentKind,
    /// Whether this agent can be a brain, worker, or both.
    #[serde(default = "default_role")]
    pub role: AgentRole,
    /// Capability tags for routing (e.g., ["security", "tests", "python"]).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Cost tier for estimation.
    #[serde(default = "default_cost_tier")]
    pub cost_tier: CostTier,
    /// Rate limit window duration, if known.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_duration_serde"
    )]
    pub rate_limit_window: Option<Duration>,
    /// Human-review policy for delegations to this agent.
    #[serde(default)]
    pub review: AgentReviewPolicy,

    /// Per-agent display metadata (short handle, display name). Optional;
    /// defaults applied by `effective_handle`.
    #[serde(default)]
    pub display: DisplayConfig,

    /// Command dispatch / vendor-ext wiring. Optional; defaults to
    /// `prompt_text` dispatch with no vendor-ext ingest or response.
    #[serde(default)]
    pub commands: CommandsConfig,

    /// Permission-bypass levers. Replaces the three flat `skip_permissions*`
    /// fields; those remain for backward compatibility and are consulted by
    /// `effective_permissions` when this block is left at default.
    #[serde(default)]
    pub permissions: PermissionsConfig,

    /// Per-kind profile wiring override (spec D9). When absent, defaults
    /// derive from `kind` via `ProfileStrategy::for_kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileConfig>,

    /// Enables bypass mode for this agent. When true, `skip_permissions_args`
    /// (if any) are appended at spawn, `skip_permissions_session_mode` (if
    /// set) is applied after session creation, and any ACP permission
    /// requests are auto-approved. See
    /// `docs/superpowers/specs/2026-04-14-spur-acp-skip-permissions-design.md`
    /// for the full mechanism. Default: false.
    #[serde(default)]
    pub skip_permissions: bool,

    /// Spawn-time CLI args appended to `args` when `skip_permissions = true`.
    /// Use for agents whose bypass is a command-line flag
    /// (e.g. `["--trust-all-tools"]` for kiro-cli,
    /// `["--dangerously-skip-permissions"]` for claude direct).
    /// Default: empty.
    #[serde(default)]
    pub skip_permissions_args: Vec<String>,

    /// ACP session mode to set via `set_session_mode` right after
    /// `new_session`, when `skip_permissions = true`. Use for agents that
    /// expose bypass as an ACP session mode (claude-code-acp →
    /// `"bypassPermissions"`). Non-fatal if the agent rejects the mode:
    /// L2 auto-approve still catches any permission calls.
    /// Default: None.
    #[serde(default)]
    pub skip_permissions_session_mode: Option<String>,

    /// Task-routing descriptor for delegation decisions. See
    /// `docs/spur/agent-config.md` and the delegation-framework spec.
    #[serde(default)]
    pub delegation: DelegationDescriptor,
}

impl AgentConfig {
    /// Construct an AgentConfig with all-default sub-tables and empty
    /// command/args, identified by `name`. Used by the TUI's fallback
    /// path (agent referenced but not listed in `.spur/config.toml`)
    /// and by test fixtures that need a minimal config stub.
    pub fn with_defaults(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: String::new(),
            args: Vec::new(),
            additional_directories: Vec::new(),
            transport: crate::types::TransportKind::Acp,
            kind: crate::types::AgentKind::Generic,
            role: crate::types::AgentRole::Both,
            capabilities: Vec::new(),
            cost_tier: crate::types::CostTier::Medium,
            rate_limit_window: None,
            review: AgentReviewPolicy::default(),
            display: DisplayConfig::default(),
            commands: CommandsConfig::default(),
            permissions: PermissionsConfig::default(),
            profile: None,
            skip_permissions: false,
            skip_permissions_args: Vec::new(),
            skip_permissions_session_mode: None,
            delegation: DelegationDescriptor::default(),
        }
    }

    /// The effective permissions for this agent, merging the legacy flat
    /// `skip_permissions*` fields with the newer `[permissions]` nested
    /// block. Precedence: if the nested block has ANY non-default value
    /// (`skip`, `args`, or `session_mode`), it wins entirely. Otherwise the
    /// flat fields are promoted into a `PermissionsConfig`.
    ///
    /// The flat fields are retained for one release cycle for back-compat.
    /// New configs should write the nested form.
    pub fn effective_permissions(&self) -> PermissionsConfig {
        let nested_is_default = !self.permissions.skip
            && self.permissions.args.is_empty()
            && self.permissions.session_mode.is_none();
        if nested_is_default {
            PermissionsConfig {
                skip: self.skip_permissions,
                args: self.skip_permissions_args.clone(),
                session_mode: self.skip_permissions_session_mode.clone(),
            }
        } else {
            self.permissions.clone()
        }
    }

    /// The short handle used as `/handle:cmd` on collision and as the
    /// key under which an agent's commands register. Prefers
    /// `display.handle` when set, otherwise falls back to
    /// `name.to_lowercase()`.
    pub fn effective_handle(&self) -> String {
        self.display
            .handle
            .clone()
            .unwrap_or_else(|| self.name.to_lowercase())
    }

    /// Args to pass when spawning this agent. Concatenates `args` with the
    /// effective `permissions.args` iff `permissions.skip` is true. Single
    /// source of truth for `spur-core`'s spawn paths — do not read
    /// `self.args` directly when spawning.
    pub fn effective_args(&self) -> Vec<String> {
        let mut out = self.args.clone();
        let perms = self.effective_permissions();
        if perms.skip {
            out.extend(perms.args.iter().cloned());
        }
        out
    }
}

fn is_valid_agent_additional_directory(path: &std::path::Path) -> bool {
    path.is_absolute()
}

pub fn validate_agent_additional_directories_absolute(
    agent_name: &str,
    additional_directories: &[PathBuf],
) -> anyhow::Result<()> {
    for path in additional_directories {
        if !is_valid_agent_additional_directory(path) {
            anyhow::bail!(
                "agent '{agent_name}' additional_directories entry is not absolute: {}",
                path.display()
            );
        }
    }
    Ok(())
}

pub(crate) fn sanitize_agent_additional_directories(config: &mut SpurConfig) {
    for agent in &mut config.agents.entries {
        let agent_name = agent.name.clone();
        agent.additional_directories.retain(|path| {
            if is_valid_agent_additional_directory(path) {
                true
            } else {
                tracing::warn!(
                    agent = %agent_name,
                    path = %path.display(),
                    "agent additional_directories entry is not absolute; skipping"
                );
                false
            }
        });
    }
}

fn default_role() -> AgentRole {
    AgentRole::Worker
}

fn default_cost_tier() -> CostTier {
    CostTier::Medium
}

/// Per-agent human-review policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReviewPolicy {
    /// When true, the orchestrator gates every delegation to this agent
    /// on a human review. Default: false.
    #[serde(default)]
    pub review_required: bool,
    /// How long to wait for a human decision before applying the default.
    /// Serialized as `review_timeout_secs` (integer seconds) in TOML.
    #[serde(
        default = "default_review_timeout",
        rename = "review_timeout_secs",
        with = "duration_secs_serde"
    )]
    pub review_timeout: Duration,
    /// What to apply on timeout.
    /// Default: `TimeoutFallback::Reject { reason: "review timeout" }`.
    #[serde(default = "default_review_timeout_default")]
    pub review_timeout_default: TimeoutFallback,
    /// Cap on `ReviewDecision::Retry` loops. Default: 3.
    #[serde(default = "default_max_review_retries")]
    pub max_review_retries: u32,
}

impl Default for AgentReviewPolicy {
    fn default() -> Self {
        Self {
            review_required: false,
            review_timeout: default_review_timeout(),
            review_timeout_default: default_review_timeout_default(),
            max_review_retries: default_max_review_retries(),
        }
    }
}

fn default_review_timeout() -> Duration {
    Duration::from_secs(30 * 60)
}

fn default_review_timeout_default() -> TimeoutFallback {
    TimeoutFallback::Reject {
        reason: "review timeout".into(),
    }
}

fn default_max_review_retries() -> u32 {
    3
}

mod duration_secs_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_secs)
    }
}

/// Runtime-level SPUR configuration knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SpurRuntimeConfig {
    /// When true, automatically merge approved plans and create PRs.
    /// Default: false (opt-in).
    pub auto_merge_approved_plans: bool,
    /// When false, disables loop scheduler sweeps at startup.
    /// Default: true.
    pub loops_enabled: bool,
    /// When true, starts the loop scheduler in globally paused mode.
    /// Default: false.
    pub pause_all_loops: bool,
    /// Grace period before startup quarantines stale `spur:plan-pending`
    /// persisted plan epics. Default: 1 hour.
    pub plan_pending_grace_secs: u64,
    /// Duration of an in-flight dispatch lease. Default: 10 minutes.
    pub dispatch_lease_secs: u64,
    /// Cadence used by the orchestrator to renew dispatch leases.
    /// Default: 200 seconds (one third of the default lease).
    pub dispatch_lease_heartbeat_secs: u64,
}

fn default_plan_pending_grace_secs() -> u64 {
    60 * 60
}

fn default_dispatch_lease_secs() -> u64 {
    600
}

fn default_dispatch_lease_heartbeat_secs() -> u64 {
    default_dispatch_lease_secs() / 3
}

fn default_loops_enabled() -> bool {
    true
}

impl Default for SpurRuntimeConfig {
    fn default() -> Self {
        Self {
            auto_merge_approved_plans: false,
            loops_enabled: default_loops_enabled(),
            pause_all_loops: false,
            plan_pending_grace_secs: default_plan_pending_grace_secs(),
            dispatch_lease_secs: default_dispatch_lease_secs(),
            dispatch_lease_heartbeat_secs: default_dispatch_lease_heartbeat_secs(),
        }
    }
}

/// Global SPUR configuration (from ~/.spur/config.toml + .spur/config.toml).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BotConfig {
    pub telegram: TelegramBotConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramBotConfig {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub operator_user_id: Option<i64>,
    pub poll_timeout_secs: u64,
    pub request_timeout_secs: Option<u64>,
    pub draft_streaming: bool,
    pub max_requests_per_second: u32,
}

impl Default for TelegramBotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            operator_user_id: None,
            poll_timeout_secs: 30,
            request_timeout_secs: None,
            draft_streaming: false,
            max_requests_per_second: 20,
        }
    }
}

/// Cloud context-service endpoint for external package code tools.
/// Built-in default points to the stable production custom domain.
/// Override in `[context_service]` in `.spur/config.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextServiceAuthMode {
    /// Preserve the configured legacy unauthenticated endpoint.
    #[default]
    None,
    /// Use a stored human OAuth bearer on the exact OAuth route.
    #[serde(rename = "oauth_bearer", alias = "o_auth_bearer")]
    OAuthBearer,
    /// Use a stored personal API key on the exact API-key route.
    ApiKey,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextServiceConfig {
    /// Cloud context-service API URL. Defaults to the production endpoint.
    pub url: String,
    /// Explicit non-secret authentication mode.
    pub auth_mode: ContextServiceAuthMode,
    /// Credential profile selected for context-service operation.
    pub profile: String,
    /// Optional non-secret API-key public identifier hint.
    pub public_id_hint: Option<String>,
    /// Legacy bearer token accepted while old configuration migrates.
    ///
    /// This field is deserialized for compatibility but is never serialized.
    #[serde(skip_serializing)]
    pub token: Option<String>,
}

impl std::fmt::Debug for ContextServiceConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContextServiceConfig")
            .field("url", &self.url)
            .field("auth_mode", &self.auth_mode)
            .field("profile", &self.profile)
            .field("public_id_hint", &self.public_id_hint)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl Default for ContextServiceConfig {
    fn default() -> Self {
        Self {
            url: "https://context.getspur.dev".to_owned(),
            auth_mode: ContextServiceAuthMode::None,
            profile: "default".to_owned(),
            public_id_hint: None,
            token: None,
        }
    }
}

/// Runtime skill projection mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillsProjectionMode {
    /// Project every bundled and accepted active pool skill (legacy full materialization).
    AllActive,
    /// Project only bundled foundation skills (skills-catalog + core SPUR skills).
    /// Remaining skills are loaded on demand via skill_navigate / skill_read.
    #[default]
    CatalogOnly,
}

impl SkillsProjectionMode {
    fn is_catalog_only(&self) -> bool {
        *self == Self::CatalogOnly
    }
}

/// Bundled skill asset resolution and runtime projection policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    /// Directory containing `assets/skills/<skill-id>/SKILL.md`-style entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundled_dir: Option<PathBuf>,
    /// Effective runtime skill set for newly reconciled brain and worker sessions.
    #[serde(default, skip_serializing_if = "SkillsProjectionMode::is_catalog_only")]
    pub projection_mode: SkillsProjectionMode,
}

impl SkillsConfig {
    fn is_default(&self) -> bool {
        self.bundled_dir.is_none() && self.projection_mode.is_catalog_only()
    }
}

/// Jev pre-compiler configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JevConfig {
    /// Whether to register `jev_compile` in the brain MCP tool registry.
    #[serde(default)]
    pub enabled: bool,
}

/// Names users may not assign to configured MCP servers. Colliding with a
/// SPUR-managed entry would shadow the fixed injection set.
pub const RESERVED_MCP_SERVER_NAMES: &[&str] = &["spur-mcp", "notebook", "spur-worker-mcp"];

/// SPUR-managed MCP server whose injection can be overridden for new sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinMcpServer {
    /// The brain-facing SPUR tool server.
    SpurMcp,
    /// The local notebook MCP proxy.
    Notebook,
    /// The curated worker-facing SPUR tool server.
    SpurWorkerMcp,
}

/// Per-server injection overrides for SPUR-managed MCP servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuiltinMcpOverridesConfig {
    /// Whether new brain sessions receive `spur-mcp`.
    pub spur_mcp_enabled: bool,
    /// Whether new brain sessions receive the notebook MCP proxy.
    pub notebook_enabled: bool,
    /// Whether new worker sessions receive `spur-worker-mcp` by default.
    pub worker_mcp_enabled: bool,
}

impl Default for BuiltinMcpOverridesConfig {
    fn default() -> Self {
        Self {
            spur_mcp_enabled: true,
            notebook_enabled: true,
            worker_mcp_enabled: true,
        }
    }
}

impl BuiltinMcpOverridesConfig {
    fn is_default(&self) -> bool {
        self.spur_mcp_enabled && self.notebook_enabled && self.worker_mcp_enabled
    }
}

/// User-configured MCP servers (`/configure mcp`). Injected into brain
/// sessions only; workers keep the curated `spur-worker-mcp` catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServersConfig {
    #[serde(default, skip_serializing_if = "BuiltinMcpOverridesConfig::is_default")]
    pub builtin_overrides: BuiltinMcpOverridesConfig,
    pub entries: Vec<McpServerEntry>,
}

impl McpServersConfig {
    pub fn is_default(&self) -> bool {
        self.builtin_overrides.is_default() && self.entries.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Unique, non-reserved identifier (`RESERVED_MCP_SERVER_NAMES`).
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub transport: McpServerTransport,
}

/// Exactly one transport per entry. The enum makes multiple transports
/// unrepresentable in Rust, while serde rejects fields from another variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase", deny_unknown_fields)]
pub enum McpServerTransport {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
        env: std::collections::HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
        headers: std::collections::HashMap<String, String>,
    },
    /// Managed read-only diagnostics served by the live Jute daemon. SPUR
    /// resolves the notebook binary and per-workspace socket at session start.
    #[serde(rename = "jute_debug")]
    JuteDebug,
}

impl McpServerEntry {
    /// Enforces the `MCP-ENTRY-VALIDATION` payload gate. Name uniqueness is
    /// enforced by upsert-replace in [`ConfigPatch::apply`].
    pub fn validate(&self) -> anyhow::Result<()> {
        let name = self.name.trim();
        anyhow::ensure!(!name.is_empty(), "mcp server name must not be empty");
        anyhow::ensure!(
            !RESERVED_MCP_SERVER_NAMES.contains(&name),
            "mcp server name '{name}' is reserved"
        );
        match &self.transport {
            McpServerTransport::Stdio { command, .. } => anyhow::ensure!(
                !command.trim().is_empty(),
                "mcp server '{}' stdio command must not be empty",
                self.name
            ),
            McpServerTransport::Http { url, .. } => {
                let url = url.trim();
                anyhow::ensure!(
                    !url.is_empty(),
                    "mcp server '{}' http url must not be empty",
                    self.name
                );
                anyhow::ensure!(
                    url.starts_with("http://") || url.starts_with("https://"),
                    "mcp server '{}' http url must use http(s)",
                    self.name
                );
            }
            McpServerTransport::JuteDebug => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpurConfig {
    #[serde(default)]
    pub brain: BrainConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub failover: FailoverConfig,
    #[serde(default)]
    pub worktree: WorktreeConfig,
    #[serde(default)]
    pub cost: CostConfig,
    #[serde(default)]
    pub pm: PmConfig,
    #[serde(default)]
    pub bot: BotConfig,
    #[serde(default)]
    pub project: Option<ProjectConfig>,
    /// Delegation dispatch tuning (Phase 1c async-first migration).
    #[serde(default)]
    pub delegation: DelegationConfig,
    /// Runtime SPUR knobs (v0e+).
    #[serde(default)]
    pub spur: SpurRuntimeConfig,
    /// Plan runtime and migration knobs.
    #[serde(default)]
    pub plan: PlanConfig,
    /// Stage-1 worker peer mailbox feature flag. Default off until explicitly
    /// opted in by production callers.
    #[serde(default)]
    pub peer_mailbox_enabled: bool,
    /// Cloud context-service endpoint for external package code tools.
    #[serde(default)]
    pub context_service: ContextServiceConfig,
    /// Bundled skill asset resolution.
    #[serde(default, skip_serializing_if = "SkillsConfig::is_default")]
    pub skills: SkillsConfig,
    /// Jev pre-compiler registration policy.
    #[serde(default)]
    pub jev: JevConfig,
    /// TUI presentation preferences (edit mode today; mouse/density/keymap
    /// in the future). Skipped on serialize when default to keep existing
    /// configs visually unchanged.
    #[serde(default, skip_serializing_if = "TuiConfig::is_default")]
    pub tui: TuiConfig,
    /// User-configured MCP servers injected into brain sessions.
    #[serde(default, skip_serializing_if = "McpServersConfig::is_default")]
    pub mcp_servers: McpServersConfig,
    /// Code-graph / embedding sidecar knobs.
    #[serde(default, skip_serializing_if = "GraphConfig::is_default")]
    pub graph: GraphConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayFsmonitorMode {
    #[default]
    Off,
    Auto,
}

impl OverlayFsmonitorMode {
    fn is_off(&self) -> bool {
        *self == Self::Off
    }
}

impl std::fmt::Display for OverlayFsmonitorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("Off"),
            Self::Auto => f.write_str("Auto (experimental)"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    /// Embedding model alias or Hugging Face id (`nomic`, `coderank`, `jina-code`).
    /// `SPUR_EMBEDDING_MODEL` still overrides when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "OverlayFsmonitorMode::is_off")]
    pub overlay_fsmonitor: OverlayFsmonitorMode,
}

impl GraphConfig {
    fn is_default(&self) -> bool {
        self.embedding_model.is_none() && self.overlay_fsmonitor.is_off()
    }
}

/// Canonical `/configure graph` embedding aliases. TUI writes only these.
pub const GRAPH_EMBEDDING_ALIASES: &[&str] = &["nomic", "coderank", "jina-code"];

/// One-section mutation for persist-then-apply `/configure` saves (`SAVE-APPLY`).
#[expect(
    clippy::large_enum_variant,
    reason = "boxing the existing Agent variant requires a cross-crate API change"
)]
#[derive(Debug, Clone)]
pub enum ConfigPatch {
    Agent {
        name: String,
        updated_entry: AgentConfig,
    },
    GraphEmbeddingModel {
        alias: String,
    },
    GraphOverlayFsmonitor(OverlayFsmonitorMode),
    TuiEditMode(EditorMode),
    TuiTheme(String),
    TuiDisablePasteBurst(bool),
    SkillsProjectionMode(SkillsProjectionMode),
    BuiltinMcpToggle {
        server: BuiltinMcpServer,
        enabled: bool,
    },
    McpServerUpsert {
        entry: McpServerEntry,
    },
    McpServerRemove {
        name: String,
    },
}

impl ConfigPatch {
    pub fn section_id(&self) -> &'static str {
        match self {
            Self::Agent { .. } => "agents",
            Self::GraphEmbeddingModel { .. } | Self::GraphOverlayFsmonitor(_) => "graph",
            Self::TuiEditMode(_) | Self::TuiTheme(_) | Self::TuiDisablePasteBurst(_) => "tui",
            Self::SkillsProjectionMode(_) => "skills",
            Self::BuiltinMcpToggle { .. }
            | Self::McpServerUpsert { .. }
            | Self::McpServerRemove { .. } => "mcp",
        }
    }

    pub fn apply(&self, cfg: &mut SpurConfig) -> anyhow::Result<()> {
        match self {
            Self::GraphEmbeddingModel { alias } => {
                if !GRAPH_EMBEDDING_ALIASES.contains(&alias.as_str()) {
                    anyhow::bail!("unsupported embedding model alias '{alias}'");
                }
                cfg.graph.embedding_model = Some(alias.clone());
            }
            Self::GraphOverlayFsmonitor(mode) => cfg.graph.overlay_fsmonitor = *mode,
            Self::TuiEditMode(mode) => cfg.tui.edit_mode = *mode,
            Self::TuiTheme(name) => cfg.tui.theme = name.clone(),
            Self::TuiDisablePasteBurst(v) => cfg.tui.disable_paste_burst = *v,
            Self::SkillsProjectionMode(mode) => cfg.skills.projection_mode = *mode,
            Self::BuiltinMcpToggle { server, enabled } => {
                let target = match server {
                    BuiltinMcpServer::SpurMcp => {
                        &mut cfg.mcp_servers.builtin_overrides.spur_mcp_enabled
                    }
                    BuiltinMcpServer::Notebook => {
                        &mut cfg.mcp_servers.builtin_overrides.notebook_enabled
                    }
                    BuiltinMcpServer::SpurWorkerMcp => {
                        &mut cfg.mcp_servers.builtin_overrides.worker_mcp_enabled
                    }
                };
                *target = *enabled;
            }
            Self::McpServerUpsert { entry } => {
                entry.validate()?;
                match cfg
                    .mcp_servers
                    .entries
                    .iter_mut()
                    .find(|configured| configured.name == entry.name)
                {
                    Some(slot) => slot.clone_from(entry),
                    None => cfg.mcp_servers.entries.push(entry.clone()),
                }
            }
            Self::McpServerRemove { name } => {
                let before = cfg.mcp_servers.entries.len();
                cfg.mcp_servers.entries.retain(|entry| entry.name != *name);
                anyhow::ensure!(
                    cfg.mcp_servers.entries.len() < before,
                    "mcp server '{name}' is not configured"
                );
            }
            Self::Agent {
                name,
                updated_entry,
            } => {
                let slot = cfg
                    .agents
                    .entries
                    .iter_mut()
                    .find(|e| e.name == *name)
                    .ok_or_else(|| anyhow::anyhow!("agent '{name}' is not configured"))?;
                slot.args = updated_entry.args.clone();
                slot.additional_directories = updated_entry.additional_directories.clone();
                slot.capabilities = updated_entry.capabilities.clone();
                slot.skip_permissions = updated_entry.skip_permissions;
                slot.skip_permissions_args = updated_entry.skip_permissions_args.clone();
                slot.skip_permissions_session_mode =
                    updated_entry.skip_permissions_session_mode.clone();
                slot.profile = updated_entry.profile.clone();
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanConfig {
    pub substrate_migration: PlanSubstrateMigrationConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanSubstrateMigrationConfig {
    /// PR2 guard: default off during staged rollout. When enabled, persisted
    /// plan reads may serve from cache if the epic audit sequence is unchanged.
    pub versioned_cache_serve: bool,
    /// PR3 guard: when true, review_task writes are non-advisory. The task
    /// decision is applied to the in-memory cache only after beads write +
    /// read-back confirms the substrate version advanced.
    pub nonadvisory_review_writes: bool,
}

/// Runtime knobs for `delegate_to_worker` / `delegate_parallel` dispatch.
///
/// Default is **pure async-first** (`inline_wait_ms = 0`): every delegation
/// falls through to the detached path and returns via the continuation
/// bridge, never via `completed_delegations` polling. A non-zero value gives
/// the worker a short inline window to complete before handing the receiver
/// to the background collector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DelegationConfig {
    /// How long the MCP handler will wait inline for a worker to respond
    /// before handing the oneshot receiver to the detached collector and
    /// returning `status: "pending"` with `continuation_will_fire: true`.
    /// Default `0` — async-first.
    pub inline_wait_ms: u64,
    /// Orchestrator-side worker branch normalization settings.
    pub normalize: DelegationNormalizeConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DelegationNormalizeConfig {
    /// When true, branch normalization git commits use `--no-verify` and
    /// `--no-gpg-sign`. Default false so user repository hooks and signing
    /// policies remain authoritative.
    pub bypass_hooks: bool,
}

/// Editing-mode preference for the TUI input bar. Stored in config so the
/// choice survives `spur tui` restarts. Maps to runtime `spur_tui::EditMode`
/// via `From` impl in `spur-tui`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorMode {
    /// Emacs-style keybindings (default; uses tui-textarea built-ins).
    #[default]
    Emacs,
    /// Vim modal editing.
    Vim,
}

/// TUI presentation preferences. New fields land here without schema churn.
/// Future additions (mouse, density, keymap) extend this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    pub edit_mode: EditorMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disable_paste_burst: bool,
    /// Active theme name. Resolved at startup via `.spur/themes/<name>.yaml`
    /// (project) → `~/.spur/themes/<name>.yaml` (user) → built-in. Defaults
    /// to `"dark"` for backwards compatibility — the dark built-in is a
    /// pixel-perfect reproduction of pre-theme TUI colors.
    #[serde(default = "default_theme_name", skip_serializing_if = "is_dark_theme")]
    pub theme: String,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            edit_mode: EditorMode::default(),
            disable_paste_burst: false,
            theme: default_theme_name(),
        }
    }
}

fn default_theme_name() -> String {
    "dark".to_string()
}

impl TuiConfig {
    /// True when this `TuiConfig` equals its `Default`. Used by
    /// `#[serde(skip_serializing_if = ...)]` on `SpurConfig.tui` so existing
    /// configs do not gain a noisy `[tui]` block on round-trip.
    pub fn is_default(&self) -> bool {
        *self == TuiConfig::default()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_dark_theme(value: &str) -> bool {
    value == "dark"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    /// Default brain agent name.
    #[serde(default = "default_brain")]
    pub default: String,
    /// Fallback brain agents, tried in order.
    #[serde(default)]
    pub fallback: Vec<String>,
    /// Additional prompt context to inject.
    #[serde(default)]
    pub prompt: BrainPromptConfig,
    /// Feature flags for the delegation framework.
    #[serde(default)]
    pub delegation: BrainDelegationConfig,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            default: default_brain(),
            fallback: vec!["kiro".to_string()],
            prompt: BrainPromptConfig::default(),
            delegation: BrainDelegationConfig::default(),
        }
    }
}

fn default_brain() -> String {
    "claude-code".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrainDelegationConfig {
    /// Which delegation framework version to use in the brain prompt.
    /// `"v1"` uses the rewritten prompt (workers block, dispatch
    /// procedure, delegation_plan guidance). `"legacy"` uses the
    /// pre-framework 5-line prose prompt. Build-aware default:
    /// debug builds default to `"v1"`; release builds default to
    /// `"legacy"` at v1 ship, flipping to `"v1"` at v2, removed at v3.
    pub framework: String,
}

impl Default for BrainDelegationConfig {
    fn default() -> Self {
        Self {
            framework: if cfg!(debug_assertions) {
                "v1".into()
            } else {
                "legacy".into()
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrainPromptConfig {
    /// Text appended to every brain prompt for this project.
    #[serde(default)]
    pub append: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub entries: Vec<AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverConfig {
    /// Minutes to wait before retrying a rate-limited agent.
    #[serde(default = "default_cooldown")]
    pub cooldown_minutes: u64,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            cooldown_minutes: default_cooldown(),
        }
    }
}

fn default_cooldown() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    /// Maximum number of concurrent worktrees.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// Hours after which stale worktrees are cleaned up.
    #[serde(default = "default_stale_hours")]
    pub stale_cleanup_hours: u64,
    /// Enable per-worker heartbeat watchdog. Default-off until the v1
    /// `_spur/heartbeat` emitter is available in production workers.
    #[serde(default = "default_worker_heartbeat_watchdog_enabled")]
    pub worker_heartbeat_watchdog_enabled: bool,
    /// Steady-state heartbeat timeout after dispatch.
    #[serde(default = "default_worker_heartbeat_timeout_secs")]
    pub worker_heartbeat_timeout_secs: u64,
    /// Startup grace before the first dispatch/heartbeat is observed.
    #[serde(default = "default_worker_heartbeat_initial_grace_secs")]
    pub worker_heartbeat_initial_grace_secs: u64,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            stale_cleanup_hours: default_stale_hours(),
            worker_heartbeat_watchdog_enabled: default_worker_heartbeat_watchdog_enabled(),
            worker_heartbeat_timeout_secs: default_worker_heartbeat_timeout_secs(),
            worker_heartbeat_initial_grace_secs: default_worker_heartbeat_initial_grace_secs(),
        }
    }
}

fn default_max_concurrent() -> usize {
    5
}

fn default_stale_hours() -> u64 {
    24
}

fn default_worker_heartbeat_watchdog_enabled() -> bool {
    false
}

fn default_worker_heartbeat_timeout_secs() -> u64 {
    90
}

fn default_worker_heartbeat_initial_grace_secs() -> u64 {
    60
}

fn default_event_replay_horizon_secs() -> u64 {
    7 * 86400
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// `tracing-subscriber` `EnvFilter` directives. `RUST_LOG` overrides if set.
    pub level: String,

    /// Per-chunk byte limit for `spur.log.YYYY-MM-DD.<n>`.
    pub max_file_bytes: u64,

    /// Number of rotated chunks to keep (`MaxFiles` argument to `file-rotate`).
    /// Active file + max_files = total chunks; total bytes ≤ (max_files+1) × max_file_bytes.
    pub max_files: usize,

    /// `tracing_appender::non_blocking` channel depth.
    pub buffered_lines_limit: usize,

    /// Per-child stderr bridge `non_blocking` channel depth.
    pub child_stderr_buffered_lines_limit: usize,

    /// Per-chunk byte limit for ACP child stderr files.
    pub child_stderr_max_bytes: u64,

    /// Number of rotated chunks per child to keep.
    pub child_stderr_max_files: usize,

    /// Total byte cap for `.spur/events/` ndjson directory.
    pub events_max_total_bytes: u64,

    /// How far back to replay NDJSON events on TUI startup, in seconds.
    /// Default 7 days. Bounds the cost of cold-start projection rehydration.
    #[serde(default = "default_event_replay_horizon_secs")]
    pub event_replay_horizon_secs: u64,

    /// If false, fall back to direct-FD stderr capture (legacy model, no rotation).
    pub child_stderr_pipe: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "warn,spur_core::orchestrator=info".to_string(),
            max_file_bytes: 8_388_608,
            max_files: 3,
            buffered_lines_limit: 8_192,
            child_stderr_buffered_lines_limit: 256,
            child_stderr_max_bytes: 2_621_440,
            child_stderr_max_files: 3,
            events_max_total_bytes: 67_108_864,
            event_replay_horizon_secs: 7 * 86400,
            child_stderr_pipe: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostConfig {
    /// Path to the SQLite cost database.
    #[serde(default = "default_db_path")]
    pub db_path: String,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
        }
    }
}

fn default_db_path() -> String {
    "~/.spur/cost.db".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PmConfig {
    #[serde(default)]
    pub github: Option<GitHubPmConfig>,
    #[serde(default)]
    pub beads: Option<BeadsPmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPmConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub use_gh_cli: bool,
    /// Repository in "owner/repo" format.
    pub repo: Option<String>,
    /// Label to auto-add to SPUR-managed issues.
    #[serde(default = "default_auto_label")]
    pub auto_label: String,

    // ── spec §9 ingest fields (all optional; defaults match constants) ──
    /// Alias for `repo` honored by `spur pm ingest github` when no
    /// positional `owner/repo` is passed. Kept as a separate key so the
    /// existing `repo` field (used by the legacy PR service) is not
    /// repurposed.
    #[serde(default)]
    pub default_repo: Option<String>,
    /// Token resolution preference; overrides the default order from §7.1.
    /// Empty / absent = use the default fallback chain.
    #[serde(default)]
    pub auth_preference: Vec<String>,
    /// Below this REST budget the governor throttles until the window resets.
    #[serde(default = "default_rate_limit_floor_rest")]
    pub rate_limit_floor_rest: u32,
    /// Below this GraphQL points budget the governor throttles until reset.
    #[serde(default = "default_rate_limit_floor_graphql_points")]
    pub rate_limit_floor_graphql_points: u32,

    /// Default ingest options applied when the CLI flags are not given.
    #[serde(default)]
    pub ingest: GitHubIngestConfig,
    /// Phase-2-only persisted token cache; Phase 1 reads this key but ignores it.
    #[serde(default)]
    pub cache: GitHubCacheConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIngestConfig {
    #[serde(default = "default_label_namespace")]
    pub label_namespace: String,
    #[serde(default = "default_ingest_page_size")]
    pub page_size: u32,
    #[serde(default = "default_auto_label_opt")]
    pub auto_label: Option<String>,
}

impl Default for GitHubIngestConfig {
    fn default() -> Self {
        Self {
            label_namespace: default_label_namespace(),
            page_size: default_ingest_page_size(),
            auto_label: default_auto_label_opt(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubCacheConfig {
    /// OS keyring service name. Phase 1 may set this; the keyring backend
    /// is wired in Phase 2.
    #[serde(default)]
    pub keyring_service: Option<String>,
}

fn default_rate_limit_floor_rest() -> u32 {
    50
}

fn default_rate_limit_floor_graphql_points() -> u32 {
    100
}

fn default_label_namespace() -> String {
    "gh".to_string()
}

fn default_ingest_page_size() -> u32 {
    25
}

fn default_auto_label_opt() -> Option<String> {
    Some("spur-managed".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadsPmConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_sync: bool,
}

fn default_true() -> bool {
    true
}

fn default_auto_label() -> String {
    "spur-managed".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
}

// ─── Duration serde helper ─────────────────────────────────────────────

mod optional_duration_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(d) => serializer.serialize_str(&format!("{}s", d.as_secs())),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            None => Ok(None),
            Some(s) => {
                let s = s.trim();
                if let Some(secs) = s.strip_suffix('s') {
                    let secs: u64 = secs.parse().map_err(serde::de::Error::custom)?;
                    Ok(Some(Duration::from_secs(secs)))
                } else if let Some(mins) = s.strip_suffix('m') {
                    let mins: u64 = mins.parse().map_err(serde::de::Error::custom)?;
                    Ok(Some(Duration::from_secs(mins * 60)))
                } else if let Some(hours) = s.strip_suffix('h') {
                    let hours: u64 = hours.parse().map_err(serde::de::Error::custom)?;
                    Ok(Some(Duration::from_secs(hours * 3600)))
                } else {
                    Err(serde::de::Error::custom(
                        "expected duration like '30s', '5m', or '2h'",
                    ))
                }
            }
        }
    }
}

/// Embedded seed template. Parsed by `load_seed_template()`. Source of
/// truth is `crates/spur-acp/src/seed_agents.toml`.
const SEED_TOML: &str = include_str!("../seed_agents.toml");

/// Parse the embedded seed template. Returns the pre-known agent set
/// that `spur init` discovers on $PATH.
///
/// Errors are unreachable in production thanks to the compile-time
/// seed-template parse test below. If a
/// maintainer skips tests and commits a bad edit, users see a clear
/// diagnostic instead of a raw panic.
pub fn load_seed_template() -> AgentsConfig {
    #[derive(serde::Deserialize)]
    struct SeedFile {
        agents: AgentsConfig,
    }
    let parsed: SeedFile = toml::from_str(SEED_TOML).unwrap_or_else(|e| {
        panic!(
            "embedded seed_agents.toml failed to parse (this is a spur bug, \
             please report): {e}"
        )
    });
    parsed.agents
}

/// Atomically read-modify-write a `SpurConfig` on disk.
///
/// Reads `path`, deserializes into `SpurConfig`, applies `mutate`, serializes
/// via `toml::to_string_pretty`, writes to a `NamedTempFile` in the same
/// directory, fsyncs the file, atomically renames over `path`, and (on Unix)
/// fsyncs the parent directory so the rename's directory-entry update is
/// durable across crashes.
///
/// Errors:
/// - `path` does not exist → returns the underlying read error.
/// - `path` contains invalid TOML → returns the parse error; original file
///   is left untouched.
/// - tempfile/write/fsync/rename failures are propagated with context.
///
/// Concurrency: two concurrent callers will produce a last-rename-wins
/// outcome. This is acceptable for preference-class fields; do NOT use this
/// helper for fields requiring CAS semantics.
pub fn update_config<F>(path: &std::path::Path, mutate: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut SpurConfig),
{
    try_update_config(path, |cfg| {
        mutate(cfg);
        Ok(())
    })
}

/// Atomically read-modify-write a `SpurConfig` on disk, propagating mutate errors
/// without writing. `update_config` is the infallible-mutate wrapper.
pub fn try_update_config<F>(path: &std::path::Path, mutate: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut SpurConfig) -> anyhow::Result<()>,
{
    use anyhow::Context;
    use std::io::Write;

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read config at {}", path.display()))?;
    let mut cfg: SpurConfig =
        toml::from_str(&raw).with_context(|| format!("parse config at {}", path.display()))?;
    mutate(&mut cfg)?;
    let serialized = toml::to_string_pretty(&cfg).context("serialize SpurConfig to TOML")?;

    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("create tempfile in {}", dir.display()))?;
    tmp.write_all(serialized.as_bytes())
        .context("write serialized config to tempfile")?;
    tmp.as_file()
        .sync_all()
        .context("fsync tempfile before rename")?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("atomic rename to {}", path.display()))?;
    // Fsync the parent directory so the rename's directory-entry update is
    // durable. POSIX `rename(2)` is atomic for visibility, but the directory
    // inode block reaching disk requires an explicit fsync on the dir.
    // Unix-only: Windows rejects opening a directory for sync.
    #[cfg(unix)]
    std::fs::File::open(dir)
        .with_context(|| format!("open config directory {} for fsync", dir.display()))?
        .sync_all()
        .with_context(|| format!("fsync config directory {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod delegation_descriptor_tests {
    use super::*;

    #[test]
    fn descriptor_deserializes_from_partial_toml() {
        let toml = r#"
            description = "test agent"
            tier = "specialist"
            good_for = ["a", "b"]
        "#;
        let d: DelegationDescriptor = toml::from_str(toml).unwrap();
        assert_eq!(d.description.as_deref(), Some("test agent"));
        assert!(matches!(d.tier, Some(Tier::Specialist)));
        assert_eq!(d.good_for, vec!["a".to_string(), "b".to_string()]);
        assert!(d.avoid_for.is_empty());
        assert!(d.inherit_defaults); // default true
    }

    #[test]
    fn descriptor_default_is_empty_and_inherits() {
        let d = DelegationDescriptor::default();
        assert!(d.description.is_none());
        assert!(d.good_for.is_empty());
        assert!(d.inherit_defaults);
    }

    #[test]
    fn agent_config_parses_delegation_sub_table() {
        let toml = r#"
            name = "claude-x"
            command = "claude"
            transport = "acp"

            [delegation]
            description = "custom claude variant"
            good_for = ["one-offs"]
        "#;
        let cfg: AgentConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            cfg.delegation.description.as_deref(),
            Some("custom claude variant")
        );
        assert_eq!(cfg.delegation.good_for, vec!["one-offs".to_string()]);
    }

    #[test]
    fn agent_config_without_delegation_block_uses_defaults() {
        let toml = r#"
            name = "bare"
            command = "bare"
            transport = "acp"
        "#;
        let cfg: AgentConfig = toml::from_str(toml).unwrap();
        assert!(cfg.delegation.description.is_none());
        assert!(cfg.delegation.inherit_defaults);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_context_auth_mode_round_trips_canonical_spelling() {
        let config = ContextServiceConfig {
            url: "https://context.example.test".to_owned(),
            auth_mode: ContextServiceAuthMode::OAuthBearer,
            profile: "default".to_owned(),
            public_id_hint: None,
            token: None,
        };
        let encoded = toml::to_string(&config).expect("serialize OAuth context config");
        assert!(encoded.contains("auth_mode = \"oauth_bearer\""));

        let decoded: ContextServiceConfig =
            toml::from_str(&encoded).expect("deserialize OAuth context config");
        assert_eq!(decoded.auth_mode, ContextServiceAuthMode::OAuthBearer);
    }

    #[test]
    fn effective_handle_prefers_display_handle() {
        let mut cfg = AgentConfig::with_defaults("ClaudeCode");
        cfg.display.handle = Some("cc".into());
        assert_eq!(cfg.effective_handle(), "cc");
    }

    #[test]
    fn effective_handle_falls_back_to_lowercased_name() {
        let cfg = AgentConfig::with_defaults("ClaudeCode");
        assert_eq!(cfg.effective_handle(), "claudecode");
    }

    #[test]
    fn seed_template_parses_and_covers_shipped_agents() {
        let seeds = load_seed_template();
        const EXPECTED_SEED_AGENTS: &[&str] = &[
            "kiro",
            "claude-code",
            "codex-bin",
            "codex",
            "gemini",
            "opencode",
        ];
        assert!(
            seeds.entries.len() >= EXPECTED_SEED_AGENTS.len(),
            "seed template must have ≥{} agents, got {}",
            EXPECTED_SEED_AGENTS.len(),
            seeds.entries.len()
        );
        let names: Vec<_> = seeds.entries.iter().map(|a| a.name.as_str()).collect();
        for expected in EXPECTED_SEED_AGENTS {
            assert!(
                names.contains(expected),
                "missing seed agent: {expected} (got {names:?})"
            );
        }
    }

    #[test]
    fn seed_template_codex_has_static_commands() {
        let seeds = load_seed_template();
        let codex = seeds
            .entries
            .iter()
            .find(|a| a.name == "codex")
            .expect("codex should be in seed template");
        assert!(
            !codex.commands.static_commands.is_empty(),
            "codex must have at least one static command (proves Spec 2)"
        );
    }

    #[test]
    fn seed_template_codex_uses_agentclientprotocol_adapter_1_9_0() {
        let seeds = load_seed_template();
        let codex = seeds
            .entries
            .iter()
            .find(|agent| agent.name == "codex")
            .expect("codex should be in seed template");

        assert_eq!(
            codex.effective_args(),
            vec![
                "--yes".to_owned(),
                "@agentclientprotocol/codex-acp@1.9.0".to_owned(),
            ]
        );
    }

    #[test]
    fn seed_template_claude_code_uses_claude_agent_acp_0_54_1() {
        // Live probe 2026-07-13: adapter 0.54.1 advertises model + effort
        // (thought_level) configOptions and agent profile selection. Bump
        // deliberately with probe_acp_capabilities -- not via @latest.
        let seeds = load_seed_template();
        let claude = seeds
            .entries
            .iter()
            .find(|agent| agent.name == "claude-code")
            .expect("claude-code should be in seed template");

        assert_eq!(claude.kind, crate::types::AgentKind::ClaudeCodeAcp);
        assert_eq!(
            claude.effective_args(),
            vec![
                "--yes".to_owned(),
                "@agentclientprotocol/claude-agent-acp@0.54.1".to_owned(),
            ]
        );
    }

    #[test]
    fn seed_template_kimi_uses_yolo_via_l2_auto_approve() {
        // kimi requires `-y` BEFORE the `acp` subcommand and relies on the
        // L2 auto-approve path (orchestrator drops `permission_tx` when
        // `effective_permissions().skip == true`) for the residual
        // shell-command permission prompt that `-y` does NOT suppress.
        // This locks both the argv order and the L2 opt-in — a
        // future edit that breaks either should fail this test.
        let seeds = load_seed_template();
        let kimi = seeds
            .entries
            .iter()
            .find(|a| a.name == "kimi")
            .expect("kimi should be in seed template");
        assert_eq!(kimi.kind, crate::types::AgentKind::Kimi);
        assert_eq!(
            kimi.effective_args(),
            vec!["-y".to_string(), "--afk".to_string(), "acp".to_string()],
            "`-y --afk` must precede `acp`: `-y` is the yolo flag, `--afk` \
             is required on kimi 1.40.0+ because `-y` alone silences \
             agent_message_chunk in ACP mode (empirically verified via \
             scripts/simulate_kimi_acp.py)"
        );
        assert!(
            kimi.effective_permissions().skip,
            "kimi must opt into L2 auto-approve via permissions.skip = true"
        );
        // Validator must accept kimi (R3 is a warning, not fatal) and the
        // single error reported must be the expected R3 — proving the
        // mechanism is intentionally L2-only.
        let errs = crate::config::validate_agent_config(kimi)
            .expect_err("expected R3 warning since mechanism is L2 auto-approve only");
        assert_eq!(errs.len(), 1, "only R3 should fire: {errs:?}");
        assert!(!errs[0].is_fatal(), "R3 must remain non-fatal");
        assert!(
            matches!(
                errs[0],
                crate::config::ConfigError::SkipPermissionsNoExplicitMechanism { .. }
            ),
            "expected R3 SkipPermissionsNoExplicitMechanism, got {:?}",
            errs[0]
        );
    }

    #[test]
    fn seed_template_grok_uses_agent_scoped_always_approve() {
        let seeds = load_seed_template();
        let grok = seeds
            .entries
            .iter()
            .find(|a| a.name == "grok")
            .expect("grok should be in seed template");

        assert_eq!(grok.kind, crate::types::AgentKind::Grok);
        assert_eq!(
            grok.effective_args(),
            vec![
                "--no-auto-update".to_string(),
                "agent".to_string(),
                "--always-approve".to_string(),
                "stdio".to_string(),
            ],
            "Grok auto-approve must be scoped to the `agent` subcommand"
        );
        assert!(
            !grok.effective_permissions().skip,
            "Grok must retain Spur's interactive permission channel as a fallback"
        );
    }

    #[test]
    fn seed_template_gemini_uses_gemini_kind() {
        let seeds = load_seed_template();
        let gemini = seeds
            .entries
            .iter()
            .find(|a| a.name == "gemini")
            .expect("gemini should be in seed template");
        assert_eq!(gemini.kind, crate::types::AgentKind::Gemini);
    }

    #[test]
    fn seed_template_opencode_starts_acp_without_cli_model_arg() {
        // OpenCode accepts `-m` for run mode, but `opencode -m <model> acp`
        // is rejected before ACP initialize. ACP model selection is a
        // session config option, so the seed must spawn only the ACP server.
        let seeds = load_seed_template();
        let opencode = seeds
            .entries
            .iter()
            .find(|a| a.name == "opencode")
            .expect("opencode should be in seed template");
        assert_eq!(opencode.kind, crate::types::AgentKind::OpenCode);
        assert_eq!(opencode.effective_args(), vec!["acp".to_string()]);
    }

    #[test]
    fn seed_template_pi_uses_pi_acp_binary() {
        // Live probe 2026-09-17: pi-acp 0.0.33 advertises model +
        // thought_level configOptions, modes, and available_commands.
        // The adapter spawns `pi --mode rpc`; argv stays empty.
        let seeds = load_seed_template();
        let pi = seeds
            .entries
            .iter()
            .find(|a| a.name == "pi")
            .expect("pi should be in seed template");
        assert_eq!(pi.kind, crate::types::AgentKind::Pi);
        assert_eq!(pi.command, "pi-acp");
        assert!(pi.effective_args().is_empty());
        assert_eq!(pi.transport, crate::types::TransportKind::Acp);
    }

    #[test]
    fn brain_delegation_framework_defaults_per_build() {
        // Empty [brain.delegation] block → build-aware default.
        let toml = r#"
            [brain]
            default = "claude-code"
        "#;
        let cfg: BrainConfig = toml::from_str(toml).unwrap();
        let expected = if cfg!(debug_assertions) {
            "v1"
        } else {
            "legacy"
        };
        assert_eq!(cfg.delegation.framework, expected);
    }

    #[test]
    fn brain_delegation_framework_explicit_v1() {
        let toml = r#"
            default = "claude-code"
            [delegation]
            framework = "v1"
        "#;
        let cfg: BrainConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.delegation.framework, "v1");
    }

    #[test]
    fn seed_template_passes_validator() {
        let seeds = load_seed_template();
        for agent in &seeds.entries {
            let errs = crate::config::validate_agent_config(agent);
            let fatal: Vec<_> = errs
                .err()
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_fatal())
                .collect();
            assert!(
                fatal.is_empty(),
                "seed agent `{}` has fatal validator errors: {fatal:?}",
                agent.name
            );
        }
    }

    #[test]
    fn spur_runtime_defaults_auto_merge_to_false() {
        let cfg: SpurConfig = toml::from_str("").unwrap();
        assert!(!cfg.spur.auto_merge_approved_plans);
        assert!(cfg.spur.loops_enabled);
        assert!(!cfg.spur.pause_all_loops);
        assert_eq!(cfg.spur.plan_pending_grace_secs, 60 * 60);
        assert_eq!(cfg.spur.dispatch_lease_secs, 600);
        assert_eq!(cfg.spur.dispatch_lease_heartbeat_secs, 200);
    }

    #[test]
    fn plan_substrate_migration_versioned_cache_serve_defaults_off() {
        let cfg: SpurConfig = toml::from_str("").unwrap();
        assert!(!cfg.plan.substrate_migration.versioned_cache_serve);
        assert!(!cfg.plan.substrate_migration.nonadvisory_review_writes);

        let parsed: SpurConfig = toml::from_str("[plan.substrate_migration]\n").unwrap();
        assert!(!parsed.plan.substrate_migration.versioned_cache_serve);
        assert!(!parsed.plan.substrate_migration.nonadvisory_review_writes);
    }

    #[test]
    fn parses_plan_substrate_migration_versioned_cache_serve() {
        let cfg: SpurConfig = toml::from_str(
            r#"
            [plan.substrate_migration]
            versioned_cache_serve = true
            nonadvisory_review_writes = true
            "#,
        )
        .unwrap();
        assert!(cfg.plan.substrate_migration.versioned_cache_serve);
        assert!(cfg.plan.substrate_migration.nonadvisory_review_writes);
    }

    #[test]
    fn spur_config_default_has_peer_mailbox_disabled() {
        let cfg = SpurConfig::default();
        assert!(!cfg.peer_mailbox_enabled);

        let parsed: SpurConfig = toml::from_str("").unwrap();
        assert!(!parsed.peer_mailbox_enabled);
    }

    #[test]
    fn worktree_defaults_have_heartbeat_watchdog_disabled() {
        let cfg = SpurConfig::default();
        assert!(!cfg.worktree.worker_heartbeat_watchdog_enabled);
        assert_eq!(cfg.worktree.worker_heartbeat_timeout_secs, 90);
        assert_eq!(cfg.worktree.worker_heartbeat_initial_grace_secs, 60);

        let parsed: SpurConfig = toml::from_str("").unwrap();
        assert!(!parsed.worktree.worker_heartbeat_watchdog_enabled);
        assert_eq!(parsed.worktree.worker_heartbeat_timeout_secs, 90);
        assert_eq!(parsed.worktree.worker_heartbeat_initial_grace_secs, 60);
    }

    #[test]
    fn spur_runtime_parses_auto_merge_true() {
        let cfg: SpurConfig = toml::from_str(
            r#"
            [spur]
            auto_merge_approved_plans = true
            loops_enabled = false
            pause_all_loops = true
            plan_pending_grace_secs = 10
            dispatch_lease_secs = 30
            dispatch_lease_heartbeat_secs = 10
            "#,
        )
        .unwrap();
        assert!(cfg.spur.auto_merge_approved_plans);
        assert!(!cfg.spur.loops_enabled);
        assert!(cfg.spur.pause_all_loops);
        assert_eq!(cfg.spur.plan_pending_grace_secs, 10);
        assert_eq!(cfg.spur.dispatch_lease_secs, 30);
        assert_eq!(cfg.spur.dispatch_lease_heartbeat_secs, 10);
    }

    #[test]
    fn parses_log_section_with_defaults() {
        let toml_str = r#"
[log]
level = "warn,spur_core::orchestrator=info"
"#;
        let cfg: SpurConfig = toml::from_str(toml_str).expect("valid config");
        assert_eq!(cfg.log.level, "warn,spur_core::orchestrator=info");
        assert_eq!(cfg.log.max_file_bytes, 8_388_608); // 8 MB
        assert_eq!(cfg.log.max_files, 3);
        assert_eq!(cfg.log.buffered_lines_limit, 8_192);
        assert_eq!(cfg.log.child_stderr_buffered_lines_limit, 256);
        assert_eq!(cfg.log.child_stderr_max_bytes, 2_621_440); // 2.5 MB
        assert_eq!(cfg.log.child_stderr_max_files, 3);
        assert_eq!(cfg.log.events_max_total_bytes, 67_108_864); // 64 MB
        assert!(cfg.log.child_stderr_pipe);
    }

    #[test]
    fn log_config_default_event_replay_horizon_is_seven_days() {
        let cfg = LogConfig::default();
        assert_eq!(cfg.event_replay_horizon_secs, 7 * 86400);
    }

    #[test]
    fn parses_empty_config_uses_log_defaults() {
        let cfg: SpurConfig = toml::from_str("").expect("empty config valid");
        assert_eq!(cfg.log.level, "warn,spur_core::orchestrator=info");
        assert_eq!(cfg.log.max_file_bytes, 8_388_608);
        assert_eq!(cfg.log.child_stderr_buffered_lines_limit, 256);
    }

    #[test]
    fn context_service_defaults_to_production_endpoint() {
        let cfg = SpurConfig::default();
        assert_eq!(cfg.context_service.url, "https://context.getspur.dev");
        assert_eq!(cfg.context_service.token, None);

        let parsed: SpurConfig = toml::from_str("").expect("empty config valid");
        assert_eq!(parsed.context_service.url, cfg.context_service.url);
        assert_eq!(parsed.context_service.token, None);
    }

    #[test]
    fn parses_context_service_override() {
        let cfg: SpurConfig = toml::from_str(
            r#"
            [context_service]
            url = "https://context.example.test"
            token = "secret-token"
            "#,
        )
        .expect("context_service section should parse");

        assert_eq!(cfg.context_service.url, "https://context.example.test");
        assert_eq!(cfg.context_service.token.as_deref(), Some("secret-token"));
    }

    #[test]
    fn skills_projection_modes_round_trip() {
        for (serialized_mode, expected) in [
            ("all_active", SkillsProjectionMode::AllActive),
            ("catalog_only", SkillsProjectionMode::CatalogOnly),
        ] {
            let config: SpurConfig = toml::from_str(&format!(
                "[skills]\nprojection_mode = \"{serialized_mode}\"\n"
            ))
            .expect("projection mode should parse");
            assert_eq!(config.skills.projection_mode, expected);

            let serialized = toml::to_string(&config).expect("config should serialize");
            let round_trip: SpurConfig =
                toml::from_str(&serialized).expect("serialized config should parse");
            assert_eq!(round_trip.skills.projection_mode, expected);
        }
    }

    #[test]
    fn skills_projection_mode_defaults_to_catalog_only_when_absent() {
        let empty: SpurConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(
            empty.skills.projection_mode,
            SkillsProjectionMode::CatalogOnly
        );

        let skills_section: SpurConfig =
            toml::from_str("[skills]\n").expect("empty skills section should parse");
        assert_eq!(
            skills_section.skills.projection_mode,
            SkillsProjectionMode::CatalogOnly
        );

        assert_eq!(
            SpurConfig::default().skills.projection_mode,
            SkillsProjectionMode::CatalogOnly
        );
    }

    #[test]
    fn jev_section_round_trips() {
        let config: SpurConfig =
            toml::from_str("[jev]\nenabled = true\n").expect("jev section should parse");
        assert!(config.jev.enabled);

        let serialized = toml::to_string(&config).expect("config should serialize");
        let round_trip: SpurConfig =
            toml::from_str(&serialized).expect("serialized config should parse");
        assert!(round_trip.jev.enabled);
    }

    #[test]
    fn jev_defaults_to_disabled_when_absent() {
        let empty: SpurConfig = toml::from_str("").expect("empty config should parse");
        assert!(!empty.jev.enabled);

        let skills_only: SpurConfig =
            toml::from_str("[skills]\nprojection_mode = \"catalog_only\"\n")
                .expect("skills-only config should parse");
        assert!(!skills_only.jev.enabled);

        assert!(!SpurConfig::default().jev.enabled);
    }

    #[test]
    fn graph_overlay_fsmonitor_parses_modes_and_defaults_off() {
        let missing: SpurConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(missing.graph.overlay_fsmonitor, OverlayFsmonitorMode::Off);

        for (serialized_mode, expected, display) in [
            ("off", OverlayFsmonitorMode::Off, "Off"),
            ("auto", OverlayFsmonitorMode::Auto, "Auto (experimental)"),
        ] {
            let config: SpurConfig = toml::from_str(&format!(
                "[graph]\noverlay_fsmonitor = \"{serialized_mode}\"\n"
            ))
            .expect("overlay fsmonitor mode should parse");
            assert_eq!(config.graph.overlay_fsmonitor, expected);
            assert_eq!(expected.to_string(), display);
        }
    }

    #[test]
    fn graph_overlay_fsmonitor_default_omits_graph_and_patch_applies_auto() {
        let serialized =
            toml::to_string(&SpurConfig::default()).expect("default config should serialize");
        assert!(!serialized.contains("[graph]"));

        let mut cfg = SpurConfig::default();
        let patch = ConfigPatch::GraphOverlayFsmonitor(OverlayFsmonitorMode::Auto);
        assert_eq!(patch.section_id(), "graph");
        patch.apply(&mut cfg).expect("graph patch should apply");
        assert_eq!(cfg.graph.overlay_fsmonitor, OverlayFsmonitorMode::Auto);

        let serialized = toml::to_string(&cfg).expect("auto config should serialize");
        assert!(serialized.contains("[graph]"));
        assert!(serialized.contains("overlay_fsmonitor = \"auto\""));
    }
}
