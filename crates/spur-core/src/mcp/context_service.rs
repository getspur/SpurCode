use async_trait::async_trait;
use rmcp::model::{ErrorCode, ErrorData as McpError};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{json, Value};
use spur_mcp::{ToolCallContext, ToolDefinition, ToolModule, ToolResponse};
use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

const DEFAULT_SOURCE: &str = "registry:crates-io";
const DEFAULT_INDEX_SOURCE: &str = "git:custom";
const KNOWLEDGE_QUERY_VECTOR_DIMENSIONS: usize = 768;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REMOTE_ERROR_CHARS: usize = 512;
const EXTERNAL_CATALOG: &str = "external_catalog";
const EXTERNAL_CODE_SEARCH: &str = "external_code_search";
const EXTERNAL_CODE_READ: &str = "external_code_read";
const EXTERNAL_CODE_CALLERS: &str = "external_code_callers";
const EXTERNAL_CODE_CALLEES: &str = "external_code_callees";
const EXTERNAL_KNOWLEDGE_CONTEXT: &str = "external_knowledge_context";
const EXTERNAL_INDEX: &str = "external_index";
const EXTERNAL_INDEX_STATUS: &str = "external_index_status";
const TOOL_NAMES: [&str; 8] = [
    EXTERNAL_CATALOG,
    EXTERNAL_CODE_SEARCH,
    EXTERNAL_CODE_READ,
    EXTERNAL_CODE_CALLERS,
    EXTERNAL_CODE_CALLEES,
    EXTERNAL_KNOWLEDGE_CONTEXT,
    EXTERNAL_INDEX,
    EXTERNAL_INDEX_STATUS,
];

/// Runtime authentication for one context-service proxy.
///
/// Secret-bearing variants intentionally redact their debug representation.
#[derive(Clone)]
pub enum ContextServiceAuth {
    /// Send no authentication header to the unauthenticated MCP routes.
    None,
    /// Send a bearer token to the exact OAuth MCP route.
    OAuthBearer(SecretString),
    /// Send a personal key to the exact API-key MCP route.
    ApiKey(SecretString),
}

impl fmt::Debug for ContextServiceAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("ContextServiceAuth::None"),
            Self::OAuthBearer(_) => {
                formatter.write_str("ContextServiceAuth::OAuthBearer([REDACTED])")
            }
            Self::ApiKey(_) => formatter.write_str("ContextServiceAuth::ApiKey([REDACTED])"),
        }
    }
}

/// A validated origin suitable for requests carrying context-service credentials.
///
/// Production origins must use HTTPS. Numeric loopback hosts may use HTTP for
/// isolated development and tests. User information, query strings, fragments,
/// and non-root paths are rejected so authenticated route selection stays exact.
#[derive(Clone, Debug)]
pub struct AuthenticatedServiceOrigin(reqwest::Url);

impl AuthenticatedServiceOrigin {
    /// Validates an authenticated context-service origin.
    pub fn parse(value: &str) -> Result<Self, ContextServiceClientError> {
        let url = parse_authenticated_endpoint(value)?;
        if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
            return Err(ContextServiceClientError::InvalidAuthenticatedOrigin);
        }
        Ok(Self(url))
    }
}

/// Construction failure for an authenticated context-service proxy.
#[derive(Debug, thiserror::Error)]
pub enum ContextServiceClientError {
    /// The supplied authenticated URL was not a safe service origin.
    #[error("invalid authenticated context-service origin")]
    InvalidAuthenticatedOrigin,
    /// The hardened HTTP client could not be built.
    #[error("could not build authenticated context-service client")]
    HttpClient(#[source] reqwest::Error),
}

#[derive(Clone)]
pub struct ContextServiceClient {
    client: reqwest::Client,
    code_endpoint: String,
    knowledge_endpoint: String,
    auth: ContextServiceAuth,
}

impl fmt::Debug for ContextServiceClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextServiceClient")
            .field("code_endpoint", &"[REDACTED URL]")
            .field("knowledge_endpoint", &"[REDACTED URL]")
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl ContextServiceClient {
    /// Creates a proxy with one explicit, mutually exclusive auth mode.
    pub fn new(
        base_url: impl Into<String>,
        auth: ContextServiceAuth,
    ) -> Result<Self, ContextServiceClientError> {
        let base_url = base_url.into();
        let (origin, route, client) = match &auth {
            ContextServiceAuth::None => {
                let origin = reqwest::Url::parse(&base_url)
                    .map_err(|_error| ContextServiceClientError::InvalidAuthenticatedOrigin)?;
                let client = reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .expect("reqwest client with static timeout configuration should build");
                (origin, "mcp", client)
            }
            ContextServiceAuth::OAuthBearer(_) | ContextServiceAuth::ApiKey(_) => {
                let origin = AuthenticatedServiceOrigin::parse(&base_url)?;
                let route = match &auth {
                    ContextServiceAuth::OAuthBearer(_) => "mcp/oauth",
                    ContextServiceAuth::ApiKey(_) => "mcp/api-key",
                    ContextServiceAuth::None => unreachable!("authenticated variants matched"),
                };
                (origin.0, route, hardened_http_client()?)
            }
        };
        Ok(Self {
            client,
            code_endpoint: origin
                .join(&format!("{route}/code"))
                .map(|url| url.to_string())
                .map_err(|_error| ContextServiceClientError::InvalidAuthenticatedOrigin)?,
            knowledge_endpoint: origin
                .join(&format!("{route}/knowledge"))
                .map(|url| url.to_string())
                .map_err(|_error| ContextServiceClientError::InvalidAuthenticatedOrigin)?,
            auth,
        })
    }

    /// Compatibility constructor for legacy optional bearer configuration.
    pub fn with_optional_token(
        base_url: impl Into<String>,
        bearer_token: Option<String>,
    ) -> Result<Self, ContextServiceClientError> {
        let base_url = base_url.into();
        let auth = match normalize_secret(bearer_token) {
            Some(token) => ContextServiceAuth::OAuthBearer(token),
            None => ContextServiceAuth::None,
        };
        Self::new(base_url, auth)
    }

    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("SPUR_CONTEXT_SERVICE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        let bearer_token = std::env::var("SPUR_CONTEXT_SERVICE_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Self::with_optional_token(base_url, bearer_token).ok()
    }

    /// Calls one context-service tool and returns its decoded JSON payload.
    pub(crate) async fn call_value(&self, name: &str, args: Value) -> Result<Value, McpError> {
        let endpoint = self.endpoint_for(name)?;
        let mut request = self
            .client
            .post(endpoint)
            .json(&json!({ "tool": name, "args": args }));
        match &self.auth {
            ContextServiceAuth::None => {}
            ContextServiceAuth::OAuthBearer(token) => {
                request = request.bearer_auth(token.expose_secret());
            }
            ContextServiceAuth::ApiKey(key) => {
                request = request.header("X-SPUR-API-Key", key.expose_secret());
            }
        }

        let response = request.send().await.map_err(|_error| {
            McpError::internal_error("context service request failed".to_owned(), None)
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(McpError::internal_error(
                format!("context service HTTP {status}"),
                None,
            ));
        }

        let value = response.json::<Value>().await.map_err(|error| {
            McpError::internal_error(
                format!("context service response was not valid JSON: {error}"),
                None,
            )
        })?;

        if let Some(error) = lambda_error_envelope(&value, &self.auth) {
            return Err(error);
        }

        Ok(value)
    }

    fn endpoint_for(&self, name: &str) -> Result<&str, McpError> {
        match name {
            EXTERNAL_CATALOG
            | EXTERNAL_CODE_SEARCH
            | EXTERNAL_CODE_READ
            | EXTERNAL_CODE_CALLERS
            | EXTERNAL_CODE_CALLEES
            | EXTERNAL_INDEX
            | EXTERNAL_INDEX_STATUS => Ok(&self.code_endpoint),
            EXTERNAL_KNOWLEDGE_CONTEXT => Ok(&self.knowledge_endpoint),
            _ => Err(McpError::invalid_params(
                format!("unknown context service tool `{name}`"),
                None,
            )),
        }
    }
}

fn parse_authenticated_endpoint(value: &str) -> Result<reqwest::Url, ContextServiceClientError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_error| ContextServiceClientError::InvalidAuthenticatedOrigin)?;
    let loopback_http = url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|host| host.is_loopback());
    if (url.scheme() != "https" && !loopback_http)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ContextServiceClientError::InvalidAuthenticatedOrigin);
    }
    Ok(url)
}

fn hardened_http_client() -> Result<reqwest::Client, ContextServiceClientError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(ContextServiceClientError::HttpClient)
}

fn normalize_secret(value: Option<String>) -> Option<SecretString> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

#[async_trait]
impl ToolModule for ContextServiceClient {
    fn tools(&self) -> Vec<ToolDefinition> {
        tool_definitions()
    }

    async fn call(
        &self,
        ctx: ToolCallContext<'_>,
        name: &str,
        args: Value,
    ) -> Result<ToolResponse, McpError> {
        let value = self.call_value(name, args).await?;
        Ok(ToolResponse::json_text(ctx.request_id_value(), value))
    }
}

fn lambda_error_envelope(value: &Value, auth: &ContextServiceAuth) -> Option<McpError> {
    let error = value.get("error")?;
    let code = i32::try_from(error.get("code")?.as_i64()?).ok()?;
    let message = redact_and_bound(error.get("message")?.as_str()?, auth);
    Some(McpError::new(ErrorCode(code), message, None))
}

fn redact_and_bound(message: &str, auth: &ContextServiceAuth) -> String {
    let redacted = match auth {
        ContextServiceAuth::None => message.to_owned(),
        ContextServiceAuth::OAuthBearer(secret) | ContextServiceAuth::ApiKey(secret) => {
            message.replace(secret.expose_secret(), "[REDACTED]")
        }
    };
    redacted.chars().take(MAX_REMOTE_ERROR_CHARS).collect()
}

pub(crate) fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        external_catalog_def(),
        external_code_search_def(),
        external_code_read_def(),
        external_code_callers_def(),
        external_code_callees_def(),
        external_knowledge_context_def(),
        external_index_def(),
        external_index_status_def(),
    ]
}

pub(crate) fn is_tool_name(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

fn external_catalog_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_CATALOG.to_owned(),
        description:
            "Browse indexed external packages, revisions, file tree entries, and file symbols."
                .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "package": {
                    "type": "string",
                    "description": "Optional package name. Omit to list indexed packages."
                },
                "revision": {
                    "type": "string",
                    "description": "Exact version or SHA for file tree or symbol descent."
                },
                "ref": {
                    "type": "string",
                    "description": "Branch or tag name. Alternative to revision."
                },
                "path": {
                    "type": "string",
                    "description": "Directory prefix or exact file path inside the package revision."
                },
                "name_filter": {
                    "type": "string",
                    "description": "Package discovery: case-insensitive name substrings or keywords (all terms), with one edit or adjacent transposition allowed for terms of at least 3 characters. Empty/whitespace lists packages; punctuation-only matches none. File symbols: case-sensitive substring filter."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 50
                },
                "cursor": {
                    "type": "string",
                    "description": "Opaque cursor returned by a previous external_catalog response."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_code_search_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_CODE_SEARCH.to_owned(),
        description: "Search symbols in an indexed external package revision.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["query", "package"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Symbol name, pattern, or qualified name."
                },
                "package": {
                    "type": "string",
                    "description": "Package name, for example serde or tokio."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "revision": {
                    "type": "string",
                    "description": "Exact version or SHA. Omit for latest."
                },
                "ref": {
                    "type": "string",
                    "description": "Branch or tag name. Alternative to revision."
                },
                "symbol_kind": {
                    "type": "string",
                    "description": "Optional symbol kind filter such as function, struct, or trait."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 20
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_code_read_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_CODE_READ.to_owned(),
        description: "Read source for one external package symbol selector.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["selector"],
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "External selector such as pkg:serde@1.0.152::serde::de::Deserialize."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Surrounding context lines."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_code_callers_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_CODE_CALLERS.to_owned(),
        description: "List symbols that call the requested external package symbol.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["selector"],
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "External selector such as pkg:serde@1.0.152::serde::de::Deserialize."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "include_unresolved": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include cross-package labeled edges."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_code_callees_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_CODE_CALLEES.to_owned(),
        description: "List symbols called by the requested external package symbol.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["selector"],
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "External selector such as pkg:serde@1.0.152::serde::de::Deserialize."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "include_unresolved": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include cross-package labeled edges."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_knowledge_context_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_KNOWLEDGE_CONTEXT.to_owned(),
        description: "Retrieve a structured evidence pack for a natural-language question about an indexed external package.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["query", "package"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language query, for example how to deserialize JSON with serde."
                },
                "package": {
                    "type": "string",
                    "description": "Package name, for example serde or tokio."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_SOURCE,
                    "description": "Package source, for example registry:crates-io or git:github.com/..."
                },
                "revision": {
                    "type": "string",
                    "description": "Exact version or SHA. Omit for latest."
                },
                "ref": {
                    "type": "string",
                    "description": "Branch or tag name. Alternative to revision."
                },
                "scope": {
                    "type": "string",
                    "enum": ["code", "docs", "all"],
                    "default": "all"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 8
                },
                "query_vec": {
                    "type": "array",
                    "items": { "type": "number" },
                    "minItems": KNOWLEDGE_QUERY_VECTOR_DIMENSIONS,
                    "maxItems": KNOWLEDGE_QUERY_VECTOR_DIMENSIONS,
                    "description": "Optional precomputed query embedding. When omitted, retrieval gracefully degrades to BM25-only."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_index_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_INDEX.to_owned(),
        description: "Queue on-demand indexing for a fetchable external package source.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["package", "revision", "source_url"],
            "properties": {
                "package": {
                    "type": "string",
                    "description": "Package name, for example serde or tokio."
                },
                "revision": {
                    "type": "string",
                    "description": "Version, branch, tag, or SHA to index."
                },
                "source_url": {
                    "type": "string",
                    "description": "Fetchable git or tarball URL for the source."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["git", "tarball"],
                    "description": "Optional source fetch strategy. When omitted it is inferred from source_url."
                },
                "source": {
                    "type": "string",
                    "default": DEFAULT_INDEX_SOURCE,
                    "description": "Catalog source namespace for this package revision."
                },
                "force": {
                    "type": "boolean",
                    "default": false,
                    "description": "Bypass the warm-path catalog hit check."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_index_status_def() -> ToolDefinition {
    ToolDefinition {
        name: EXTERNAL_INDEX_STATUS.to_owned(),
        description: "Return the queued indexing job status for a job_id.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["job_id"],
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "Job identifier returned by external_index."
                }
            },
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // Env mutation is process-global and requires an unsafe block on current Rust.
mod tests {
    use super::*;
    use axum::extract::{OriginalUri, State};
    use axum::http::header::AUTHORIZATION;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{any, post};
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone)]
    struct MockService {
        response: Value,
        observed: Arc<Mutex<Option<oneshot::Sender<ObservedRequest>>>>,
    }

    #[derive(Clone)]
    struct CountingMockService {
        response: Value,
        observed_paths: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Debug)]
    struct ObservedRequest {
        authorization: Option<String>,
        api_key: Option<String>,
        path: String,
        body: Value,
    }

    #[test]
    fn authenticated_origin_rejects_insecure_or_non_root_urls() {
        for invalid in [
            "http://context.example.test",
            "https://user@example.test",
            "https://example.test/service",
            "https://example.test/?region=test",
            "https://example.test/#fragment",
        ] {
            assert!(
                ContextServiceClient::new(
                    invalid,
                    ContextServiceAuth::ApiKey("secret".to_owned().into()),
                )
                .is_err(),
                "authenticated origin should reject {invalid}"
            );
        }
        assert!(
            ContextServiceClient::with_optional_token(
                "http://context.example.test/custom-route",
                Some("legacy-secret".to_owned()),
            )
            .is_err(),
            "legacy bearer endpoints must also reject non-loopback HTTP"
        );
    }

    #[tokio::test]
    async fn authenticated_client_does_not_follow_redirects() {
        let (target_tx, target_rx) = oneshot::channel();
        let target_sender = Arc::new(Mutex::new(Some(target_tx)));
        let target_app = Router::new().fallback(any({
            let target_sender = Arc::clone(&target_sender);
            move || {
                let target_sender = Arc::clone(&target_sender);
                async move {
                    if let Some(sender) = target_sender
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                    {
                        let _ = sender.send(());
                    }
                    StatusCode::OK
                }
            }
        }));
        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect target");
        let target_addr = target_listener.local_addr().expect("redirect target addr");
        tokio::spawn(async move {
            axum::serve(target_listener, target_app)
                .await
                .expect("serve redirect target");
        });

        let redirect_app = Router::new().route(
            "/mcp/api-key/code",
            post(move || async move {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("location", format!("http://{target_addr}/captured"))],
                )
            }),
        );
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect service");
        let redirect_addr = redirect_listener.local_addr().expect("redirect addr");
        tokio::spawn(async move {
            axum::serve(redirect_listener, redirect_app)
                .await
                .expect("serve redirect service");
        });

        let client = ContextServiceClient::new(
            format!("http://{redirect_addr}"),
            ContextServiceAuth::ApiKey("api-key-secret".to_owned().into()),
        )
        .expect("loopback HTTP is allowed for isolated tests");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        let error = match client
            .call(ctx, "external_code_search", json!({ "query": "serde" }))
            .await
        {
            Ok(_) => panic!("redirect response should not be followed"),
            Err(error) => error,
        };

        assert!(error.message.contains("HTTP 307"));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), target_rx)
                .await
                .is_err(),
            "redirect target must not receive an authenticated request"
        );
    }

    #[tokio::test]
    async fn optional_bearer_does_not_follow_redirects_on_direct_route() {
        let (target_tx, target_rx) = oneshot::channel();
        let target_sender = Arc::new(Mutex::new(Some(target_tx)));
        let target_app = Router::new().fallback(any({
            let target_sender = Arc::clone(&target_sender);
            move || {
                let target_sender = Arc::clone(&target_sender);
                async move {
                    if let Some(sender) = target_sender
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                    {
                        let _ = sender.send(());
                    }
                    StatusCode::OK
                }
            }
        }));
        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind legacy redirect target");
        let target_addr = target_listener
            .local_addr()
            .expect("legacy redirect target addr");
        tokio::spawn(async move {
            axum::serve(target_listener, target_app)
                .await
                .expect("serve legacy redirect target");
        });

        let redirect_app = Router::new().route(
            "/mcp/oauth/code",
            post(move || async move {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("location", format!("http://{target_addr}/captured"))],
                )
            }),
        );
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind legacy redirect service");
        let redirect_addr = redirect_listener
            .local_addr()
            .expect("legacy redirect addr");
        tokio::spawn(async move {
            axum::serve(redirect_listener, redirect_app)
                .await
                .expect("serve legacy redirect service");
        });

        let client = ContextServiceClient::with_optional_token(
            format!("http://{redirect_addr}"),
            Some("legacy-secret".to_owned()),
        )
        .expect("loopback legacy bearer endpoint");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        let error = match client
            .call(ctx, "external_code_search", json!({ "query": "serde" }))
            .await
        {
            Ok(_) => panic!("legacy bearer redirect should not be followed"),
            Err(error) => error,
        };

        assert!(error.message.contains("HTTP 307"));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), target_rx)
                .await
                .is_err(),
            "legacy redirect target must not receive a bearer request"
        );
    }

    #[tokio::test]
    async fn envelope_success_path_produces_tool_response() {
        let (base_url, observed) = spawn_mock_service(json!({
            "matches": [
                { "selector": "pkg:serde@1.0.0::serde::Deserialize" }
            ]
        }))
        .await;
        let client = ContextServiceClient::new(
            base_url,
            ContextServiceAuth::OAuthBearer("secret-token".to_owned().into()),
        )
        .expect("loopback authenticated origin");
        let request_id = json!("req-1");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            Some(&request_id),
        );

        let response = client
            .call(
                ctx,
                "external_code_search",
                json!({ "query": "Deserialize", "package": "serde" }),
            )
            .await;

        assert!(
            response.is_ok(),
            "success envelope should produce ToolResponse"
        );
        let observed = observed.await.expect("mock service should receive request");
        assert_eq!(
            observed.authorization.as_deref(),
            Some("Bearer secret-token")
        );
        assert_eq!(observed.api_key, None);
        assert_eq!(observed.path, "/mcp/oauth/code");
        assert_eq!(
            observed.body,
            json!({
                "tool": "external_code_search",
                "args": { "query": "Deserialize", "package": "serde" }
            })
        );
    }

    #[tokio::test]
    async fn api_key_uses_exact_route_and_header_without_bearer() {
        let (base_url, observed) = spawn_mock_service(json!({ "matches": [] })).await;
        let client = ContextServiceClient::new(
            base_url,
            ContextServiceAuth::ApiKey("spur_test_public_secret".to_owned().into()),
        )
        .expect("loopback authenticated origin");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        client
            .call(
                ctx,
                "external_code_search",
                json!({ "query": "Deserialize" }),
            )
            .await
            .expect("API-key request should succeed");

        let observed = observed.await.expect("mock service should receive request");
        assert_eq!(observed.authorization, None);
        assert_eq!(observed.api_key.as_deref(), Some("spur_test_public_secret"));
        assert_eq!(observed.path, "/mcp/api-key/code");
    }

    #[tokio::test]
    async fn omits_authorization_header_when_token_is_not_configured() {
        let (base_url, observed) = spawn_mock_service(json!({
            "matches": []
        }))
        .await;
        let client = ContextServiceClient::new(base_url, ContextServiceAuth::None)
            .expect("legacy anonymous client");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        client
            .call(
                ctx,
                "external_code_search",
                json!({ "query": "Deserialize", "package": "serde" }),
            )
            .await
            .expect("success envelope should produce ToolResponse");

        let observed = observed.await.expect("mock service should receive request");
        assert_eq!(observed.authorization, None);
        assert_eq!(observed.api_key, None);
        assert_eq!(observed.path, "/mcp/code");
    }

    #[tokio::test]
    async fn context_service_routes_external_tools_directly() {
        let (base_url, observed_paths) = spawn_counting_mock_service(json!({})).await;
        let tool_routes = [
            (EXTERNAL_CATALOG, "code"),
            (EXTERNAL_CODE_SEARCH, "code"),
            (EXTERNAL_CODE_READ, "code"),
            (EXTERNAL_CODE_CALLERS, "code"),
            (EXTERNAL_CODE_CALLEES, "code"),
            (EXTERNAL_KNOWLEDGE_CONTEXT, "knowledge"),
            (EXTERNAL_INDEX, "code"),
            (EXTERNAL_INDEX_STATUS, "code"),
        ];
        let auth_routes = [
            (
                ContextServiceAuth::OAuthBearer("oauth-secret".to_owned().into()),
                "/mcp/oauth",
            ),
            (
                ContextServiceAuth::ApiKey("api-key-secret".to_owned().into()),
                "/mcp/api-key",
            ),
            (ContextServiceAuth::None, "/mcp"),
        ];
        let expected_request_count = auth_routes.len() * tool_routes.len();
        let mut expected_paths = Vec::new();

        for (auth, auth_route) in auth_routes {
            let client = ContextServiceClient::new(base_url.clone(), auth)
                .expect("loopback context-service origin");
            for (tool, service) in tool_routes {
                client
                    .call_value(tool, json!({}))
                    .await
                    .expect("direct external-tool request should succeed");
                expected_paths.push(format!("{auth_route}/{service}"));
            }
        }

        let observed_paths = observed_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(*observed_paths, expected_paths);
        assert_eq!(observed_paths.len(), expected_request_count);
    }

    #[tokio::test]
    async fn context_service_rejects_unknown_external_tool_without_post() {
        let (base_url, observed_paths) = spawn_counting_mock_service(json!({})).await;
        let client = ContextServiceClient::new(
            base_url,
            ContextServiceAuth::OAuthBearer("oauth-secret".to_owned().into()),
        )
        .expect("loopback context-service origin");

        let error = client
            .call_value("external_future_tool", json!({}))
            .await
            .expect_err("the closed route classifier must reject unknown tools");

        assert_eq!(error.code, ErrorCode(-32602));
        assert!(observed_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
    }

    #[tokio::test]
    async fn error_envelope_maps_to_mcp_error() {
        let (base_url, _observed) = spawn_mock_service(json!({
            "error": {
                "code": -32602,
                "message": "field 'package' is required"
            }
        }))
        .await;
        let client = ContextServiceClient::new(
            base_url,
            ContextServiceAuth::OAuthBearer("secret-token".to_owned().into()),
        )
        .expect("loopback authenticated origin");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        let err = match client
            .call(
                ctx,
                "external_code_search",
                json!({ "query": "Deserialize" }),
            )
            .await
        {
            Ok(_) => panic!("error envelope should map to McpError"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorCode(-32602));
        assert_eq!(err.message, "field 'package' is required");
    }

    #[tokio::test]
    async fn optional_bearer_uses_direct_oauth_code_route() {
        let (base_url, observed) = spawn_mock_service(json!({ "matches": [] })).await;
        let client =
            ContextServiceClient::with_optional_token(base_url, Some("legacy-secret".to_owned()))
                .expect("loopback legacy bearer endpoint");
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        client
            .call(ctx, "external_code_search", json!({ "query": "serde" }))
            .await
            .expect("legacy request should succeed");

        let observed = observed.await.expect("mock service should receive request");
        assert_eq!(observed.path, "/mcp/oauth/code");
        assert_eq!(
            observed.authorization.as_deref(),
            Some("Bearer legacy-secret")
        );
    }

    #[tokio::test]
    async fn remote_errors_and_debug_output_redact_credentials() {
        let secret = "oauth-secret-value";
        let (base_url, _observed) = spawn_mock_service(json!({
            "error": {
                "code": -32603,
                "message": format!("{secret} {}", "x".repeat(700))
            }
        }))
        .await;
        let client = ContextServiceClient::new(
            base_url,
            ContextServiceAuth::OAuthBearer(secret.to_owned().into()),
        )
        .expect("loopback authenticated origin");
        assert!(!format!("{client:?}").contains(secret));
        let ctx = ToolCallContext::new(
            spur_mcp::ServerKind::Brain,
            spur_mcp::ToolAuthority::Brain,
            None,
            None,
        );

        let error = match client
            .call(ctx, "external_code_search", json!({ "query": "serde" }))
            .await
        {
            Ok(_) => panic!("remote error envelope should fail"),
            Err(error) => error,
        };
        assert!(!error.message.contains(secret));
        assert!(error.message.chars().count() <= MAX_REMOTE_ERROR_CHARS);
    }

    #[test]
    fn debug_output_does_not_expose_url_userinfo() {
        let client = ContextServiceClient::new(
            "https://user:url-secret@example.test",
            ContextServiceAuth::None,
        )
        .expect("legacy anonymous client");
        assert!(!format!("{client:?}").contains("url-secret"));
    }

    #[test]
    fn from_env_returns_none_when_unconfigured() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prev_url = std::env::var_os("SPUR_CONTEXT_SERVICE_URL");
        let prev_token = std::env::var_os("SPUR_CONTEXT_SERVICE_TOKEN");

        // SAFETY: `ENV_LOCK` serializes these process-global mutations with all
        // environment-mutating tests in this module.
        unsafe {
            std::env::remove_var("SPUR_CONTEXT_SERVICE_URL");
            std::env::remove_var("SPUR_CONTEXT_SERVICE_TOKEN");
        }
        let configured = ContextServiceClient::from_env();
        restore_env("SPUR_CONTEXT_SERVICE_URL", prev_url);
        restore_env("SPUR_CONTEXT_SERVICE_TOKEN", prev_token);

        assert!(configured.is_none());
    }

    #[test]
    fn from_env_accepts_url_without_token() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prev_url = std::env::var_os("SPUR_CONTEXT_SERVICE_URL");
        let prev_token = std::env::var_os("SPUR_CONTEXT_SERVICE_TOKEN");

        // SAFETY: `ENV_LOCK` serializes these process-global mutations with all
        // environment-mutating tests in this module.
        unsafe {
            std::env::set_var("SPUR_CONTEXT_SERVICE_URL", "https://context.example.test");
            std::env::remove_var("SPUR_CONTEXT_SERVICE_TOKEN");
        }
        let configured = ContextServiceClient::from_env();
        restore_env("SPUR_CONTEXT_SERVICE_URL", prev_url);
        restore_env("SPUR_CONTEXT_SERVICE_TOKEN", prev_token);

        assert!(configured.is_some());
    }

    async fn spawn_mock_service(response: Value) -> (String, oneshot::Receiver<ObservedRequest>) {
        let (tx, rx) = oneshot::channel();
        let state = MockService {
            response,
            observed: Arc::new(Mutex::new(Some(tx))),
        };
        let app = Router::new()
            .route("/mcp/code", post(mock_handler))
            .route("/mcp/knowledge", post(mock_handler))
            .route("/mcp/oauth/code", post(mock_handler))
            .route("/mcp/oauth/knowledge", post(mock_handler))
            .route("/mcp/api-key/code", post(mock_handler))
            .route("/mcp/api-key/knowledge", post(mock_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock context service");
        let addr = listener.local_addr().expect("mock local addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock context service");
        });
        (format!("http://{addr}"), rx)
    }

    async fn spawn_counting_mock_service(response: Value) -> (String, Arc<Mutex<Vec<String>>>) {
        let observed_paths = Arc::new(Mutex::new(Vec::new()));
        let state = CountingMockService {
            response,
            observed_paths: Arc::clone(&observed_paths),
        };
        let app = Router::new()
            .route("/", post(counting_mock_handler))
            .route("/mcp/oauth", post(counting_mock_handler))
            .route("/mcp/api-key", post(counting_mock_handler))
            .route("/mcp/code", post(counting_mock_handler))
            .route("/mcp/knowledge", post(counting_mock_handler))
            .route("/mcp/oauth/code", post(counting_mock_handler))
            .route("/mcp/oauth/knowledge", post(counting_mock_handler))
            .route("/mcp/api-key/code", post(counting_mock_handler))
            .route("/mcp/api-key/knowledge", post(counting_mock_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counting mock context service");
        let addr = listener.local_addr().expect("counting mock local addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve counting mock context service");
        });
        (format!("http://{addr}"), observed_paths)
    }

    async fn mock_handler(
        State(state): State<MockService>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let api_key = headers
            .get("x-spur-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if let Some(sender) = state
            .observed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(ObservedRequest {
                authorization,
                api_key,
                path: uri.path().to_owned(),
                body,
            });
        }
        Json(state.response)
    }

    async fn counting_mock_handler(
        State(state): State<CountingMockService>,
        OriginalUri(uri): OriginalUri,
    ) -> Json<Value> {
        state
            .observed_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(uri.path().to_owned());
        Json(state.response)
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            // SAFETY: callers hold `ENV_LOCK` until restoration completes.
            Some(value) => unsafe { std::env::set_var(name, value) },
            // SAFETY: callers hold `ENV_LOCK` until restoration completes.
            None => unsafe { std::env::remove_var(name) },
        }
    }
}
