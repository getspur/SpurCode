use std::collections::BTreeSet;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub(crate) use crate::api_key_authorizer::ApiKeyAuthContext;

#[cfg(test)]
use std::collections::BTreeMap;

const DEFAULT_RESOURCE_SERVER_ID: &str = "urn:spur:context-service";
pub(crate) const OAUTH_PATH: &str = "/mcp/oauth";
pub(crate) const DISCOVERY_PATH: &str = "/.well-known/spur-context-service";
pub(crate) const LOGIN_PATH: &str = "/auth/login";
pub(crate) const API_KEY_MCP_PATH: &str = "/mcp/api-key";
pub(crate) const API_KEY_MANAGEMENT_PATH: &str = "/auth/api-keys";

const OAUTH_SPLIT_PATHS: [&str; 2] = ["/mcp/oauth/code", "/mcp/oauth/knowledge"];
const API_KEY_SPLIT_PATHS: [&str; 2] = ["/mcp/api-key/code", "/mcp/api-key/knowledge"];

const EXTERNAL_TOOL_SCOPE_SUFFIXES: [(&str, &str); 8] = [
    ("external_catalog", "external.read"),
    ("external_code_search", "external.read"),
    ("external_code_read", "external.read"),
    ("external_code_callers", "external.read"),
    ("external_code_callees", "external.read"),
    ("external_knowledge_context", "external.read"),
    ("external_index", "external.index"),
    ("external_index_status", "external.status"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestRoute {
    OAuth,
    Discovery,
    Login,
    ApiKeyMcp,
    ApiKeyCreate,
    ApiKeyList,
    ApiKeyRevoke,
    ReservedUnavailable,
    Legacy,
}

impl RequestRoute {
    #[cfg(test)]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::Discovery => "discovery",
            Self::Login => "login",
            Self::ApiKeyMcp => "api_key_mcp",
            Self::ApiKeyCreate => "api_key_create",
            Self::ApiKeyList => "api_key_list",
            Self::ApiKeyRevoke => "api_key_revoke",
            Self::ReservedUnavailable => "reserved_unavailable",
            Self::Legacy => "legacy",
        }
    }

    pub(crate) const fn is_api_key(self) -> bool {
        matches!(
            self,
            Self::ApiKeyMcp | Self::ApiKeyCreate | Self::ApiKeyList | Self::ApiKeyRevoke
        )
    }
}

pub(crate) fn classify_route(path: Option<&str>, method: Option<&str>) -> RequestRoute {
    let Some(path) = path else {
        return RequestRoute::Legacy;
    };
    let Some(method) = method else {
        return if is_reserved_route_namespace(path) {
            RequestRoute::ReservedUnavailable
        } else {
            RequestRoute::Legacy
        };
    };
    match (method, path) {
        ("POST", OAUTH_PATH) => RequestRoute::OAuth,
        ("POST", path) if OAUTH_SPLIT_PATHS.contains(&path) => RequestRoute::OAuth,
        (_, path) if path.starts_with("/mcp/oauth/") => RequestRoute::ReservedUnavailable,
        ("GET", DISCOVERY_PATH) => RequestRoute::Discovery,
        (_, DISCOVERY_PATH) => RequestRoute::ReservedUnavailable,
        (_, path) if path.starts_with("/.well-known/spur-context-service/") => {
            RequestRoute::ReservedUnavailable
        }
        ("GET", LOGIN_PATH) => RequestRoute::Login,
        (_, LOGIN_PATH) => RequestRoute::ReservedUnavailable,
        (_, path) if path.starts_with("/auth/login/") => RequestRoute::ReservedUnavailable,
        ("POST", API_KEY_MCP_PATH) => RequestRoute::ApiKeyMcp,
        ("POST", path) if API_KEY_SPLIT_PATHS.contains(&path) => RequestRoute::ApiKeyMcp,
        (_, API_KEY_MCP_PATH) => RequestRoute::ReservedUnavailable,
        (_, path) if path.starts_with("/mcp/api-key/") => RequestRoute::ReservedUnavailable,
        ("POST", API_KEY_MANAGEMENT_PATH) => RequestRoute::ApiKeyCreate,
        ("GET", API_KEY_MANAGEMENT_PATH) => RequestRoute::ApiKeyList,
        (_, API_KEY_MANAGEMENT_PATH) => RequestRoute::ReservedUnavailable,
        ("DELETE", path)
            if path
                .strip_prefix("/auth/api-keys/")
                .is_some_and(|key_id| !key_id.is_empty() && !key_id.contains('/')) =>
        {
            RequestRoute::ApiKeyRevoke
        }
        (_, path) if path.starts_with("/auth/api-keys/") => RequestRoute::ReservedUnavailable,
        _ => RequestRoute::Legacy,
    }
}

fn is_reserved_route_namespace(path: &str) -> bool {
    path.strip_prefix(OAUTH_PATH)
        .is_some_and(|rest| rest.starts_with('/'))
        || [
            DISCOVERY_PATH,
            LOGIN_PATH,
            API_KEY_MCP_PATH,
            API_KEY_MANAGEMENT_PATH,
        ]
        .into_iter()
        .any(|prefix| {
            path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthScheme {
    CognitoUser,
    CognitoClient,
    Iam,
    LegacyAnonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrincipalKind {
    Human,
    Machine,
    Iam,
    Anonymous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallerIdentity {
    caller_id: String,
    scheme: AuthScheme,
    principal_kind: PrincipalKind,
}

impl CallerIdentity {
    fn new(caller_id: String, scheme: AuthScheme, principal_kind: PrincipalKind) -> Self {
        Self {
            caller_id,
            scheme,
            principal_kind,
        }
    }

    pub(crate) fn caller_id(&self) -> &str {
        &self.caller_id
    }

    #[cfg(test)]
    pub(crate) fn scheme(&self) -> AuthScheme {
        self.scheme
    }

    #[cfg(test)]
    pub(crate) fn principal_kind(&self) -> PrincipalKind {
        self.principal_kind
    }
}

pub(crate) fn legacy_anonymous_identity() -> CallerIdentity {
    CallerIdentity::new(
        "anonymous-internal".to_owned(),
        AuthScheme::LegacyAnonymous,
        PrincipalKind::Anonymous,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthDecision {
    pub(crate) identity: CallerIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthFailure {
    AuthDisabled,
    MissingContext,
    WrongRoute,
    WrongIssuer,
    WrongTokenUse,
    MissingClient,
    UnknownClient,
    DenylistedClient,
    InvalidSubject,
    InvalidExpiry,
    MalformedScope,
    UnexpectedAudience,
    MissingScope,
    NonExternalTool,
    InvalidIamIdentity,
    InvalidApiKeyContext,
    HumanManagementRequired,
    InvalidConfiguration,
}

impl AuthFailure {
    pub(crate) fn status_code(&self) -> u16 {
        match self {
            Self::MissingScope | Self::NonExternalTool | Self::HumanManagementRequired => 403,
            Self::AuthDisabled
            | Self::MissingContext
            | Self::WrongRoute
            | Self::WrongIssuer
            | Self::WrongTokenUse
            | Self::MissingClient
            | Self::UnknownClient
            | Self::DenylistedClient
            | Self::InvalidSubject
            | Self::InvalidExpiry
            | Self::MalformedScope
            | Self::UnexpectedAudience
            | Self::InvalidIamIdentity
            | Self::InvalidApiKeyContext
            | Self::InvalidConfiguration => 401,
        }
    }

    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Self::AuthDisabled => "auth_disabled",
            Self::MissingContext => "missing_context",
            Self::WrongRoute => "wrong_route",
            Self::WrongIssuer => "wrong_issuer",
            Self::WrongTokenUse => "wrong_token_use",
            Self::MissingClient => "missing_client",
            Self::UnknownClient => "unknown_client",
            Self::DenylistedClient => "denylisted_client",
            Self::InvalidSubject => "malformed_subject",
            Self::InvalidExpiry => "invalid_expiry",
            Self::MalformedScope => "malformed_scope",
            Self::UnexpectedAudience => "unexpected_audience",
            Self::MissingScope => "missing_scope",
            Self::NonExternalTool => "non_external_tool",
            Self::InvalidIamIdentity => "invalid_iam_identity",
            Self::InvalidApiKeyContext => "invalid_api_key_context",
            Self::HumanManagementRequired => "human_management_required",
            Self::InvalidConfiguration => "invalid_configuration",
        }
    }
}

pub(crate) fn authorize_api_key_tool(
    tool: &str,
    context: Option<&ApiKeyAuthContext>,
) -> Result<AuthDecision, AuthFailure> {
    let context = context.ok_or(AuthFailure::MissingContext)?;
    let required_scope = required_scope_suffix(tool).ok_or(AuthFailure::NonExternalTool)?;
    if !context
        .scopes
        .as_strings()
        .into_iter()
        .any(|scope| scope == required_scope)
    {
        return Err(AuthFailure::MissingScope);
    }
    Ok(AuthDecision {
        identity: CallerIdentity::new(
            context.owner_id.clone(),
            AuthScheme::CognitoUser,
            PrincipalKind::Human,
        ),
    })
}

pub(crate) fn authorize_key_management(
    config: &AuthConfig,
    claims: Option<&Value>,
    now_epoch_seconds: u64,
) -> Result<AuthDecision, AuthFailure> {
    config.validate()?;
    let claims = claims
        .and_then(Value::as_object)
        .ok_or(AuthFailure::MissingContext)?;
    if claims
        .get("iss")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        != Some(config.issuer.as_str())
    {
        return Err(AuthFailure::WrongIssuer);
    }
    if claims
        .get("token_use")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        != Some("access")
    {
        return Err(AuthFailure::WrongTokenUse);
    }
    let client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        .ok_or(AuthFailure::MissingClient)?;
    if config.deny_client_ids.contains(client_id) {
        return Err(AuthFailure::DenylistedClient);
    }
    if client_id != config.human_client_id {
        return Err(AuthFailure::HumanManagementRequired);
    }
    if let Some(audience) = claims.get("aud") {
        if audience
            .as_str()
            .and_then(valid_claim_value)
            .filter(|audience| *audience == config.resource_server_id)
            .is_none()
        {
            return Err(AuthFailure::UnexpectedAudience);
        }
    }
    claims
        .get("exp")
        .and_then(parse_expiry)
        .filter(|expiry| *expiry > now_epoch_seconds)
        .ok_or(AuthFailure::InvalidExpiry)?;
    let scopes = parse_scopes(
        claims
            .get("scope")
            .and_then(Value::as_str)
            .ok_or(AuthFailure::MalformedScope)?,
    )?;
    let required_scope = format!("{}/keys.manage", config.resource_server_id);
    if !scopes.contains(&required_scope) {
        return Err(AuthFailure::MissingScope);
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        .ok_or(AuthFailure::InvalidSubject)?;
    Ok(AuthDecision {
        identity: CallerIdentity::new(
            format!("cognito:user:{subject}"),
            AuthScheme::CognitoUser,
            PrincipalKind::Human,
        ),
    })
}

fn required_scope_suffix(tool: &str) -> Option<&'static str> {
    EXTERNAL_TOOL_SCOPE_SUFFIXES
        .iter()
        .find_map(|(candidate, scope)| (*candidate == tool).then_some(*scope))
}

#[derive(Debug, Clone)]
pub(crate) struct AuthConfig {
    issuer: String,
    human_client_id: String,
    m2m_client_ids: BTreeSet<String>,
    deny_client_ids: BTreeSet<String>,
    resource_server_id: String,
}

impl AuthConfig {
    pub(crate) fn new<I, D, T, U>(
        issuer: impl Into<String>,
        human_client_id: impl Into<String>,
        m2m_client_ids: I,
        deny_client_ids: D,
        resource_server_id: impl Into<String>,
    ) -> Self
    where
        I: IntoIterator<Item = T>,
        D: IntoIterator<Item = U>,
        T: AsRef<str>,
        U: AsRef<str>,
    {
        Self {
            issuer: issuer.into(),
            human_client_id: human_client_id.into(),
            m2m_client_ids: m2m_client_ids
                .into_iter()
                .map(|client_id| client_id.as_ref().trim().to_owned())
                .collect(),
            deny_client_ids: deny_client_ids
                .into_iter()
                .map(|client_id| client_id.as_ref().trim().to_owned())
                .collect(),
            resource_server_id: resource_server_id.into(),
        }
    }

    pub(crate) fn from_environment() -> Result<Option<Self>, AuthFailure> {
        if !environment_is_truthy("SPUR_COGNITO_AUTH_ENABLED") {
            return Ok(None);
        }

        let config = Self::new(
            required_environment("SPUR_COGNITO_ISSUER")?,
            required_environment("SPUR_COGNITO_HUMAN_CLIENT_ID")?,
            comma_separated_environment("SPUR_COGNITO_M2M_CLIENT_IDS")?,
            comma_separated_environment("SPUR_COGNITO_DENY_CLIENT_IDS")?,
            env::var("SPUR_COGNITO_RESOURCE_SERVER_ID")
                .unwrap_or_else(|_| DEFAULT_RESOURCE_SERVER_ID.to_owned()),
        );
        config.validate()?;
        Ok(Some(config))
    }

    fn validate(&self) -> Result<(), AuthFailure> {
        if valid_claim_value(&self.issuer).is_none()
            || valid_claim_value(&self.human_client_id).is_none()
            || valid_claim_value(&self.resource_server_id).is_none()
            || self
                .m2m_client_ids
                .iter()
                .chain(self.deny_client_ids.iter())
                .any(|client_id| valid_claim_value(client_id).is_none())
        {
            return Err(AuthFailure::InvalidConfiguration);
        }
        Ok(())
    }

    fn allows_client(&self, client_id: &str) -> bool {
        client_id == self.human_client_id || self.m2m_client_ids.contains(client_id)
    }

    fn required_scope(&self, tool: &str) -> Option<String> {
        EXTERNAL_TOOL_SCOPE_SUFFIXES
            .iter()
            .find(|(candidate, _)| *candidate == tool)
            .map(|(_, suffix)| format!("{}/{}", self.resource_server_id, suffix))
    }
}

#[cfg(test)]
pub(crate) fn external_tool_scopes() -> BTreeMap<&'static str, &'static str> {
    EXTERNAL_TOOL_SCOPE_SUFFIXES
        .into_iter()
        .map(|(tool, suffix)| {
            let scope = match suffix {
                "external.read" => "urn:spur:context-service/external.read",
                "external.index" => "urn:spur:context-service/external.index",
                "external.status" => "urn:spur:context-service/external.status",
                _ => unreachable!("the external scope policy is static"),
            };
            (tool, scope)
        })
        .collect()
}

pub(crate) fn authorize_oauth_tool(
    config: &AuthConfig,
    tool: &str,
    claims: Option<&Value>,
    now_epoch_seconds: u64,
) -> Result<AuthDecision, AuthFailure> {
    config.validate()?;
    let claims = claims
        .and_then(Value::as_object)
        .ok_or(AuthFailure::MissingContext)?;

    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        .ok_or(AuthFailure::WrongIssuer)?;
    if issuer != config.issuer {
        return Err(AuthFailure::WrongIssuer);
    }

    let token_use = claims
        .get("token_use")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        .ok_or(AuthFailure::WrongTokenUse)?;
    if token_use != "access" {
        return Err(AuthFailure::WrongTokenUse);
    }

    let client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .and_then(valid_claim_value)
        .ok_or(AuthFailure::MissingClient)?;
    if config.deny_client_ids.contains(client_id) {
        return Err(AuthFailure::DenylistedClient);
    }
    if !config.allows_client(client_id) {
        return Err(AuthFailure::UnknownClient);
    }

    if let Some(audience) = claims.get("aud") {
        let audience = audience
            .as_str()
            .and_then(valid_claim_value)
            .ok_or(AuthFailure::UnexpectedAudience)?;
        if audience != config.resource_server_id {
            return Err(AuthFailure::UnexpectedAudience);
        }
    }

    let expiry = claims
        .get("exp")
        .and_then(parse_expiry)
        .filter(|expiry| *expiry > now_epoch_seconds)
        .ok_or(AuthFailure::InvalidExpiry)?;
    let _ = expiry;

    let scopes = parse_scopes(
        claims
            .get("scope")
            .and_then(Value::as_str)
            .ok_or(AuthFailure::MalformedScope)?,
    )?;

    let identity = if client_id == config.human_client_id {
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .and_then(valid_claim_value)
            .ok_or(AuthFailure::InvalidSubject)?;
        CallerIdentity::new(
            format!("cognito:user:{subject}"),
            AuthScheme::CognitoUser,
            PrincipalKind::Human,
        )
    } else {
        CallerIdentity::new(
            format!("cognito:client:{client_id}"),
            AuthScheme::CognitoClient,
            PrincipalKind::Machine,
        )
    };

    let required_scope = config
        .required_scope(tool)
        .ok_or(AuthFailure::NonExternalTool)?;
    if !scopes.contains(&required_scope) {
        return Err(AuthFailure::MissingScope);
    }

    Ok(AuthDecision { identity })
}

pub(crate) fn authorize_oauth_tool_now(
    config: &AuthConfig,
    tool: &str,
    claims: Option<&Value>,
) -> Result<AuthDecision, AuthFailure> {
    let now_epoch_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthFailure::InvalidExpiry)?
        .as_secs();
    authorize_oauth_tool(config, tool, claims, now_epoch_seconds)
}

pub(crate) fn parse_scopes(scope: &str) -> Result<BTreeSet<String>, AuthFailure> {
    if scope.is_empty() || scope.contains('\0') {
        return Err(AuthFailure::MalformedScope);
    }

    let scopes = scope
        .split_whitespace()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| valid_claim_value(scope).is_none())
    {
        return Err(AuthFailure::MalformedScope);
    }
    Ok(scopes)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IamContext<'a> {
    pub(crate) account_id: Option<&'a str>,
    pub(crate) user_id: Option<&'a str>,
    pub(crate) user_arn: Option<&'a str>,
}

impl IamContext<'_> {
    pub(crate) fn authenticate(self) -> Result<CallerIdentity, AuthFailure> {
        if let (Some(account_id), Some(user_id)) = (
            self.account_id.and_then(valid_claim_value),
            self.user_id.and_then(valid_claim_value),
        ) {
            let principal_unique_id = user_id.split(':').next().unwrap_or_default();
            if is_aws_account_id(account_id) && valid_claim_value(principal_unique_id).is_some() {
                return Ok(CallerIdentity::new(
                    format!("iam:{account_id}:{principal_unique_id}"),
                    AuthScheme::Iam,
                    PrincipalKind::Iam,
                ));
            }
        }

        let user_arn = self
            .user_arn
            .and_then(valid_claim_value)
            .filter(|arn| is_canonical_iam_user_arn(arn))
            .ok_or(AuthFailure::InvalidIamIdentity)?;
        Ok(CallerIdentity::new(
            format!("iam:{user_arn}"),
            AuthScheme::Iam,
            PrincipalKind::Iam,
        ))
    }
}

fn environment_is_truthy(name: &str) -> bool {
    matches!(
        env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

fn required_environment(name: &str) -> Result<String, AuthFailure> {
    env::var(name)
        .ok()
        .and_then(|value| valid_claim_value(&value).map(str::to_owned))
        .ok_or(AuthFailure::InvalidConfiguration)
}

fn comma_separated_environment(name: &str) -> Result<Vec<String>, AuthFailure> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(Vec::new()),
        Ok(value) => value
            .split(',')
            .map(|item| {
                valid_claim_value(item)
                    .map(str::to_owned)
                    .ok_or(AuthFailure::InvalidConfiguration)
            })
            .collect(),
        Err(env::VarError::NotPresent) => Ok(Vec::new()),
        Err(env::VarError::NotUnicode(_)) => Err(AuthFailure::InvalidConfiguration),
    }
}

fn parse_expiry(value: &Value) -> Option<u64> {
    match value {
        Value::String(value)
            if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            value.parse().ok()
        }
        Value::String(_) => None,
        _ => value.as_u64(),
    }
}

fn valid_claim_value(value: &str) -> Option<&str> {
    let value = value.trim();
    let length = value.len();
    (1..=256)
        .contains(&length)
        .then_some(value)
        .filter(|value| {
            !value
                .chars()
                .any(|character| character == '\0' || character.is_ascii_control())
        })
}

fn is_aws_account_id(value: &str) -> bool {
    value.len() == 12 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_canonical_iam_user_arn(arn: &str) -> bool {
    let mut parts = arn.splitn(6, ':');
    matches!(
        (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ),
        (
            Some("arn"),
            Some(partition),
            Some("iam"),
            Some(""),
            Some(account_id),
            Some(resource),
        ) if !partition.is_empty()
            && is_aws_account_id(account_id)
            && resource.starts_with("user/")
            && resource.len() > "user/".len()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{json, Value};

    use super::{
        authorize_oauth_tool, classify_route, external_tool_scopes, parse_scopes, AuthConfig,
        AuthFailure, AuthScheme, IamContext, PrincipalKind, RequestRoute, API_KEY_MANAGEMENT_PATH,
        API_KEY_MCP_PATH, DISCOVERY_PATH,
    };

    fn config() -> AuthConfig {
        AuthConfig::new(
            "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "human-client",
            ["m2m-client", "other-m2m"],
            ["blocked-client"],
            "urn:spur:context-service",
        )
    }

    fn access_claims() -> serde_json::Value {
        json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "opaque-human-subject",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.read"
        })
    }

    #[test]
    fn scope_policy_covers_exactly_the_eight_external_tools() {
        let policy = external_tool_scopes();
        assert_eq!(policy.len(), 8);
        assert_eq!(
            policy.keys().copied().collect::<BTreeSet<_>>(),
            [
                "external_catalog",
                "external_code_search",
                "external_code_read",
                "external_code_callers",
                "external_code_callees",
                "external_knowledge_context",
                "external_index",
                "external_index_status",
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn exact_post_oauth_path_is_classified_and_its_child_namespace_is_reserved() {
        assert_eq!(
            classify_route(Some("/mcp/oauth"), Some("POST")),
            RequestRoute::OAuth
        );
        assert_eq!(
            classify_route(Some("/mcp/oauth"), Some("GET")),
            RequestRoute::Legacy
        );
        assert_eq!(
            classify_route(Some("/mcp/oauth/"), Some("POST")),
            RequestRoute::ReservedUnavailable
        );
    }

    #[test]
    fn exact_post_oauth_split_paths_use_oauth_classification() {
        for path in ["/mcp/oauth/code", "/mcp/oauth/knowledge"] {
            assert_eq!(
                classify_route(Some(path), Some("POST")),
                RequestRoute::OAuth,
                "OAuth split path {path} must retain OAuth classification"
            );
        }
    }

    #[test]
    fn exact_post_api_key_split_paths_use_api_key_classification() {
        for path in ["/mcp/api-key/code", "/mcp/api-key/knowledge"] {
            assert_eq!(
                classify_route(Some(path), Some("POST")),
                RequestRoute::ApiKeyMcp,
                "API-key split path {path} must retain API-key classification"
            );
        }
    }

    #[test]
    fn authenticated_split_paths_fail_closed_for_wrong_methods() {
        for path in [
            "/mcp/oauth/code",
            "/mcp/oauth/knowledge",
            "/mcp/api-key/code",
            "/mcp/api-key/knowledge",
        ] {
            for method in [None, Some("GET"), Some("PUT"), Some("DELETE")] {
                assert_eq!(
                    classify_route(Some(path), method),
                    RequestRoute::ReservedUnavailable,
                    "authenticated split path {path} must fail closed for method {method:?}"
                );
            }
        }
    }

    #[test]
    fn authenticated_namespaces_reject_unknown_and_extra_suffixes() {
        for path in [
            "/mcp/oauth/unknown",
            "/mcp/oauth/code/extra",
            "/mcp/oauth/knowledge/extra",
            "/mcp/api-key/unknown",
            "/mcp/api-key/code/extra",
            "/mcp/api-key/knowledge/extra",
        ] {
            for method in [Some("POST"), Some("GET")] {
                assert_eq!(
                    classify_route(Some(path), method),
                    RequestRoute::ReservedUnavailable,
                    "authenticated namespace path {path} must fail closed for method {method:?}"
                );
            }
        }
    }

    #[test]
    fn login_facade_reserves_exactly_get_auth_login() {
        assert_eq!(
            classify_route(Some("/auth/login"), Some("GET")).as_str(),
            "login"
        );
        for method in [None, Some("POST"), Some("PUT"), Some("DELETE")] {
            assert_eq!(
                classify_route(Some("/auth/login"), method),
                RequestRoute::ReservedUnavailable,
                "login facade must fail closed for method {method:?}"
            );
        }
        assert_eq!(
            classify_route(Some("/auth/login/"), Some("GET")),
            RequestRoute::ReservedUnavailable
        );
        assert_eq!(
            classify_route(Some("/auth/login-other"), Some("GET")),
            RequestRoute::Legacy
        );
    }

    #[test]
    fn reserved_auth_namespaces_fail_closed_when_the_method_is_missing() {
        for path in [
            DISCOVERY_PATH,
            "/.well-known/spur-context-service/child",
            "/auth/login",
            "/auth/login/child",
            API_KEY_MCP_PATH,
            "/mcp/api-key/child",
            API_KEY_MANAGEMENT_PATH,
            "/auth/api-keys/aaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(
                classify_route(Some(path), None),
                RequestRoute::ReservedUnavailable,
                "reserved path {path} must not downgrade without a method"
            );
        }
        assert_eq!(
            classify_route(Some("/mcp/api-key-legacy"), None),
            RequestRoute::Legacy
        );
    }

    #[cfg(feature = "service")]
    #[test]
    fn scope_policy_fails_closed_when_the_mcp_external_surface_drifts() {
        let policy_tools = external_tool_scopes()
            .into_keys()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let mcp_tools = crate::mcp::tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .filter(|name| name.starts_with("external_"))
            .collect::<BTreeSet<_>>();

        assert_eq!(policy_tools, mcp_tools);
    }

    #[test]
    fn scopes_are_whitespace_delimited_exact_and_duplicate_safe() {
        let scopes = parse_scopes(
            "urn:spur:context-service/external.read  urn:spur:context-service/external.read\nurn:spur:context-service/external.index",
        )
        .expect("well-formed scopes should parse");

        assert_eq!(scopes.len(), 2);
        assert!(scopes.contains("urn:spur:context-service/external.read"));
        assert!(!scopes.contains("urn:spur:context-service/external.re"));
    }

    #[test]
    fn every_external_tool_requires_its_exact_policy_scope() {
        let policy = external_tool_scopes();

        for (tool, scope) in &policy {
            let mut allowed_claims = access_claims();
            allowed_claims["scope"] = Value::String((*scope).to_owned());
            assert!(
                authorize_oauth_tool(&config(), tool, Some(&allowed_claims), 1_700_000_000).is_ok(),
                "{tool} should accept its exact scope"
            );

            let other_scope = policy
                .iter()
                .find_map(|(_, other_scope)| (*other_scope != *scope).then_some(*other_scope))
                .expect("policy has multiple distinct tools");
            let mut denied_claims = access_claims();
            denied_claims["scope"] = Value::String(other_scope.to_owned());
            assert_eq!(
                authorize_oauth_tool(&config(), tool, Some(&denied_claims), 1_700_000_000),
                Err(AuthFailure::MissingScope),
                "{tool} must not accept {other_scope}"
            );
        }
    }

    #[test]
    fn human_access_token_derives_namespaced_user_identity() {
        let decision = authorize_oauth_tool(
            &config(),
            "external_catalog",
            Some(&access_claims()),
            1_700_000_000,
        )
        .expect("human token with exact read scope should authorize");

        assert_eq!(
            decision.identity.caller_id(),
            "cognito:user:opaque-human-subject"
        );
        assert_eq!(decision.identity.scheme(), AuthScheme::CognitoUser);
        assert_eq!(decision.identity.principal_kind(), PrincipalKind::Human);
    }

    #[test]
    fn m2m_access_token_without_sub_or_aud_derives_client_identity() {
        let claims = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "m2m-client",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.index"
        });

        let decision =
            authorize_oauth_tool(&config(), "external_index", Some(&claims), 1_700_000_000)
                .expect("M2M access token should not need human claims");

        assert_eq!(decision.identity.caller_id(), "cognito:client:m2m-client");
        assert_eq!(decision.identity.scheme(), AuthScheme::CognitoClient);
        assert_eq!(decision.identity.principal_kind(), PrincipalKind::Machine);
    }

    #[test]
    fn unexpected_m2m_subject_does_not_change_client_ownership() {
        let claims = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "m2m-client",
            "sub": "unexpected-human-shaped-subject",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.index"
        });

        let decision =
            authorize_oauth_tool(&config(), "external_index", Some(&claims), 1_700_000_000)
                .expect("M2M ownership must be determined by client_id");

        assert_eq!(decision.identity.caller_id(), "cognito:client:m2m-client");
    }

    #[test]
    fn oauth_claim_failures_are_fail_closed_and_bounded() {
        let cases = [
            (None, AuthFailure::MissingContext),
            (Some(json!({})), AuthFailure::WrongIssuer),
            (
                Some(json!({
                    "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
                    "token_use": "id",
                    "client_id": "human-client",
                    "sub": "subject",
                    "exp": 2_000_000_000_u64,
                    "scope": "urn:spur:context-service/external.read"
                })),
                AuthFailure::WrongTokenUse,
            ),
            (
                Some(json!({
                    "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
                    "token_use": "access",
                    "client_id": "unknown-client",
                    "sub": "subject",
                    "exp": 2_000_000_000_u64,
                    "scope": "urn:spur:context-service/external.read"
                })),
                AuthFailure::UnknownClient,
            ),
            (
                Some(json!({
                    "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
                    "token_use": "access",
                    "client_id": "blocked-client",
                    "sub": "subject",
                    "exp": 2_000_000_000_u64,
                    "scope": "urn:spur:context-service/external.read"
                })),
                AuthFailure::DenylistedClient,
            ),
            (
                Some(json!({
                    "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
                    "token_use": "access",
                    "client_id": "human-client",
                    "sub": "subject",
                    "exp": "not-a-number",
                    "scope": "urn:spur:context-service/external.read"
                })),
                AuthFailure::InvalidExpiry,
            ),
        ];

        for (claims, expected) in cases {
            assert_eq!(
                authorize_oauth_tool(
                    &config(),
                    "external_catalog",
                    claims.as_ref(),
                    1_700_000_000
                ),
                Err(expected)
            );
        }
    }

    #[test]
    fn string_expiries_reject_malformed_negative_overflow_and_expired_values() {
        for expiry in ["not-a-number", "-1", "18446744073709551616", "1700000000"] {
            let mut claims = access_claims();
            claims["exp"] = Value::String(expiry.to_owned());

            assert_eq!(
                authorize_oauth_tool(&config(), "external_catalog", Some(&claims), 1_700_000_000),
                Err(AuthFailure::InvalidExpiry),
                "{expiry:?} must fail semantic expiry validation"
            );
        }
    }

    #[test]
    fn malformed_subject_scope_and_audience_fail_closed() {
        let invalid_subject = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "bad\u{0000}subject",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.read"
        });
        let malformed_scope = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "subject",
            "exp": 2_000_000_000_u64,
            "scope": "\u{0000}"
        });
        let unexpected_audience = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "subject",
            "aud": "different-client",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.read"
        });
        let mismatched_allowlisted_audience = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "subject",
            "aud": "other-m2m",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.read"
        });
        let matching_resource_audience = json!({
            "iss": "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_pool",
            "token_use": "access",
            "client_id": "human-client",
            "sub": "subject",
            "aud": "urn:spur:context-service",
            "exp": 2_000_000_000_u64,
            "scope": "urn:spur:context-service/external.read"
        });

        assert_eq!(
            authorize_oauth_tool(
                &config(),
                "external_catalog",
                Some(&invalid_subject),
                1_700_000_000
            ),
            Err(AuthFailure::InvalidSubject)
        );
        assert_eq!(
            authorize_oauth_tool(
                &config(),
                "external_catalog",
                Some(&malformed_scope),
                1_700_000_000
            ),
            Err(AuthFailure::MalformedScope)
        );
        assert_eq!(
            authorize_oauth_tool(
                &config(),
                "external_catalog",
                Some(&unexpected_audience),
                1_700_000_000,
            ),
            Err(AuthFailure::UnexpectedAudience)
        );
        assert!(authorize_oauth_tool(
            &config(),
            "external_catalog",
            Some(&matching_resource_audience),
            1_700_000_000,
        )
        .is_ok());
        assert_eq!(
            authorize_oauth_tool(
                &config(),
                "external_catalog",
                Some(&mismatched_allowlisted_audience),
                1_700_000_000,
            ),
            Err(AuthFailure::UnexpectedAudience)
        );
    }

    #[test]
    fn oauth_route_returns_forbidden_for_missing_exact_scope_or_non_external_tool() {
        let claims = access_claims();

        assert_eq!(
            authorize_oauth_tool(&config(), "external_index", Some(&claims), 1_700_000_000),
            Err(AuthFailure::MissingScope)
        );
        assert_eq!(
            authorize_oauth_tool(&config(), "internal_admin", Some(&claims), 1_700_000_000),
            Err(AuthFailure::NonExternalTool)
        );
        assert_eq!(AuthFailure::MissingScope.status_code(), 403);
        assert_eq!(AuthFailure::WrongIssuer.status_code(), 401);
    }

    #[test]
    fn malformed_human_subject_takes_precedence_over_scope_denial() {
        let mut claims = access_claims();
        claims["sub"] = Value::String(" \t ".to_owned());
        claims["scope"] = Value::String("urn:spur:context-service/external.status".to_owned());

        let failure =
            authorize_oauth_tool(&config(), "external_catalog", Some(&claims), 1_700_000_000)
                .expect_err("a malformed human identity must be unauthenticated");
        assert_eq!(failure, AuthFailure::InvalidSubject);
        assert_eq!(failure.status_code(), 401);
    }

    #[test]
    fn strict_iam_uses_stable_principal_prefix_and_never_source_ip() {
        let decision = IamContext {
            account_id: Some("123456789012"),
            user_id: Some("AROAXYZ:temporary-session-name"),
            user_arn: None,
        }
        .authenticate()
        .expect("stable IAM context should authenticate");
        assert_eq!(decision.caller_id(), "iam:123456789012:AROAXYZ");

        assert_eq!(
            IamContext {
                account_id: None,
                user_id: None,
                user_arn: Some("not-an-iam-user-arn"),
            }
            .authenticate(),
            Err(AuthFailure::InvalidIamIdentity)
        );
    }

    #[test]
    fn auth_scheme_and_principal_kind_keep_legacy_anonymous_distinct() {
        assert_ne!(AuthScheme::LegacyAnonymous, AuthScheme::Iam);
        assert_ne!(PrincipalKind::Anonymous, PrincipalKind::Iam);
    }
}
