//! Lean API-key authorizer contracts shared with the serving Lambda.
//!
//! This module deliberately depends only on [`crate::api_keys`] plus serde. The
//! dedicated authorizer binary includes it directly and never imports the
//! context-service library, catalog, or `DuckDB` modules.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::api_keys::{parse_api_key, verify_secret, ApiKeyScopes, ApiKeyStore, KeyEnvironment};

const PUBLIC_ID_LEN: usize = 26;
const MAX_OWNER_ID_LEN: usize = 512;
const API_KEY_ROUTE_KEY: &str = "POST /mcp/api-key";
const API_KEY_HEADER: &str = "x-spur-api-key";
const AUTHENTICATION_FAILED_BODY: &str = r#"{"error":{"code":"authentication_failed"}}"#;
const AUTHORIZER_UNAVAILABLE_BODY: &str = r#"{"error":{"code":"authorizer_unavailable"}}"#;

/// Minimal HTTP API request-authorizer event fields used by the verifier.
#[derive(Deserialize)]
pub struct ApiKeyAuthorizerRequest {
    /// API Gateway's selected route key. It is also part of the cache identity.
    #[serde(rename = "routeKey", default)]
    pub route_key: Option<String>,
    /// Configured identity-source values: raw header followed by route key.
    #[serde(rename = "identitySource", default)]
    pub identity_source: Option<Vec<String>>,
    /// Request headers. Only `X-SPUR-API-Key` is read.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for ApiKeyAuthorizerRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiKeyAuthorizerRequest")
            .field("route_key", &self.route_key)
            .field("identity_source", &"[REDACTED]")
            .field("headers", &"[REDACTED]")
            .finish()
    }
}

/// HTTP API simple authorizer response for an allow or cacheable credential deny.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiKeyAuthorizerResponse {
    #[serde(rename = "isAuthorized")]
    is_authorized: bool,
    context: ApiKeyAuthorizerResponseContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ApiKeyAuthorizerResponseContext {
    Authorized(ApiKeyAuthContext),
    Denied(ApiKeyAuthorizerDenyContext),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ApiKeyAuthorizerDenyContext {
    auth_context_version: u8,
    auth_kind: ApiKeyAuthKind,
    denial_code: &'static str,
}

impl ApiKeyAuthorizerResponse {
    fn denied() -> Self {
        Self {
            is_authorized: false,
            context: ApiKeyAuthorizerResponseContext::Denied(ApiKeyAuthorizerDenyContext {
                auth_context_version: 1,
                auth_kind: ApiKeyAuthKind::ApiKey,
                denial_code: "authentication_failed",
            }),
        }
    }
}

/// Bounded authorizer failure safe for logs and the API Gateway adapter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyAuthorizerError {
    /// Missing, malformed, unknown, mismatched, expired, or revoked credential.
    AuthenticationFailed,
    /// Store or configuration failure.
    Unavailable,
}

impl ApiKeyAuthorizerError {
    /// Domain status classification for bounded diagnostics and tests.
    ///
    /// HTTP API v2 simple authorizer responses do not carry a custom denial
    /// status; API Gateway owns the status for a serialized deny decision.
    #[must_use]
    pub const fn status_code(self) -> u16 {
        match self {
            Self::AuthenticationFailed => 401,
            Self::Unavailable => 503,
        }
    }

    /// Constant bounded domain body. Credential failures are indistinguishable.
    ///
    /// This is not emitted as a custom HTTP body by the simple-response Lambda
    /// boundary.
    #[must_use]
    pub const fn body(self) -> &'static str {
        match self {
            Self::AuthenticationFailed => AUTHENTICATION_FAILED_BODY,
            Self::Unavailable => AUTHORIZER_UNAVAILABLE_BODY,
        }
    }
}

impl fmt::Debug for ApiKeyAuthorizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthenticationFailed => "AuthenticationFailed",
            Self::Unavailable => "Unavailable",
        })
    }
}

impl fmt::Display for ApiKeyAuthorizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthenticationFailed => "API key authentication failed",
            Self::Unavailable => "API key authorizer unavailable",
        })
    }
}

impl std::error::Error for ApiKeyAuthorizerError {}

/// Authentication scheme carried in API Gateway's trusted Lambda-authorizer context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyAuthKind {
    /// A personal SPUR API key.
    ApiKey,
}

/// Version-one trusted integration context produced by the API-key authorizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyAuthContext {
    /// Wire contract version. V1 is the only accepted value.
    pub auth_context_version: u8,
    /// Authentication scheme discriminator.
    pub auth_kind: ApiKeyAuthKind,
    /// Existing queue/rate/status owner, always `cognito:user:<sub>`.
    pub owner_id: String,
    /// Non-secret public key identifier.
    pub key_id: String,
    /// Normalized API-key scopes.
    pub scopes: ApiKeyScopes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireApiKeyAuthContext {
    auth_context_version: u8,
    auth_kind: ApiKeyAuthKind,
    owner_id: String,
    key_id: String,
    scopes: String,
}

#[derive(Serialize)]
struct SerializableApiKeyAuthContext<'a> {
    auth_context_version: u8,
    auth_kind: ApiKeyAuthKind,
    owner_id: &'a str,
    key_id: &'a str,
    scopes: String,
}

impl Serialize for ApiKeyAuthContext {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerializableApiKeyAuthContext {
            auth_context_version: self.auth_context_version,
            auth_kind: self.auth_kind,
            owner_id: &self.owner_id,
            key_id: &self.key_id,
            scopes: self.scopes.as_strings().join(" "),
        }
        .serialize(serializer)
    }
}

/// Bounded context validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid API key authorizer context")]
pub struct ApiKeyContextError;

impl ApiKeyAuthContext {
    fn new(
        owner_id: String,
        key_id: String,
        scopes: ApiKeyScopes,
    ) -> Result<Self, ApiKeyContextError> {
        if !valid_owner_id(&owner_id) || !valid_public_id(&key_id) {
            return Err(ApiKeyContextError);
        }
        Ok(Self {
            auth_context_version: 1,
            auth_kind: ApiKeyAuthKind::ApiKey,
            owner_id,
            key_id,
            scopes,
        })
    }

    /// Parses and validates an untrusted API Gateway authorizer context value.
    ///
    /// # Errors
    ///
    /// Returns an error when the context shape, version, identity, key ID, or
    /// normalized scopes do not exactly match the API-key context contract.
    pub fn from_value(value: &Value) -> Result<Self, ApiKeyContextError> {
        let wire = serde_json::from_value::<WireApiKeyAuthContext>(value.clone())
            .map_err(|_| ApiKeyContextError)?;
        let scopes = ApiKeyScopes::parse(&wire.scopes.split_whitespace().collect::<Vec<_>>())
            .map_err(|_| ApiKeyContextError)?;
        if wire.auth_context_version != 1
            || !valid_owner_id(&wire.owner_id)
            || !valid_public_id(&wire.key_id)
            || scopes.as_strings().join(" ") != wire.scopes
        {
            return Err(ApiKeyContextError);
        }
        Ok(Self {
            auth_context_version: wire.auth_context_version,
            auth_kind: wire.auth_kind,
            owner_id: wire.owner_id,
            key_id: wire.key_id,
            scopes,
        })
    }

    /// Returns the existing caller/owner identifier.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// Returns the non-secret public key identifier.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

/// Authenticates one API-key request using exactly one strongly consistent lookup.
///
/// # Errors
///
/// Returns an authentication failure for an invalid request or credential, and
/// an unavailable failure when the durable store or persisted context is unsafe.
pub async fn authorize_api_key(
    request: &ApiKeyAuthorizerRequest,
    store: &dyn ApiKeyStore,
    expected_environment: KeyEnvironment,
    now_epoch_seconds: u64,
) -> Result<ApiKeyAuthorizerResponse, ApiKeyAuthorizerError> {
    let credential = credential_for_exact_route(request)?;
    let parsed =
        parse_api_key(credential).map_err(|_| ApiKeyAuthorizerError::AuthenticationFailed)?;
    if parsed.environment != expected_environment {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    }

    let record = store
        .get_key_consistent(parsed.public_id)
        .await
        .map_err(|_| ApiKeyAuthorizerError::Unavailable)?;
    let dummy_digest = [0_u8; 32];
    let digest = record.as_ref().map_or(dummy_digest.as_slice(), |record| {
        record.secret_hash.as_slice()
    });
    let secret_matches = verify_secret(&parsed, digest);
    let record = record.ok_or(ApiKeyAuthorizerError::AuthenticationFailed)?;
    if !secret_matches
        || !record.is_active_at(now_epoch_seconds)
        || record.public_id != parsed.public_id
    {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    }
    let context = ApiKeyAuthContext::new(record.owner_id, record.public_id, record.scopes)
        .map_err(|_| ApiKeyAuthorizerError::Unavailable)?;
    Ok(ApiKeyAuthorizerResponse {
        is_authorized: true,
        context: ApiKeyAuthorizerResponseContext::Authorized(context),
    })
}

/// Produces the HTTP API v2 simple response used by the dedicated Lambda boundary.
///
/// Credential failures become a serializable deny so API Gateway can cache the
/// decision. Configuration and store failures remain Lambda failures and must
/// never be cached as an allow. The setting defaults to `live`; any present
/// value other than exactly `live` or `test` fails closed before a store lookup.
///
/// # Errors
///
/// Returns an unavailable failure for invalid environment configuration or a
/// durable-store failure. Credential failures are returned as cacheable denies.
pub async fn authorize_api_key_with_environment(
    request: &ApiKeyAuthorizerRequest,
    store: &dyn ApiKeyStore,
    expected_environment: Option<&str>,
    now_epoch_seconds: u64,
) -> Result<ApiKeyAuthorizerResponse, ApiKeyAuthorizerError> {
    let expected_environment = match expected_environment.unwrap_or("live") {
        "live" => KeyEnvironment::Live,
        "test" => KeyEnvironment::Test,
        _ => return Err(ApiKeyAuthorizerError::Unavailable),
    };
    match authorize_api_key(request, store, expected_environment, now_epoch_seconds).await {
        Ok(response) => Ok(response),
        Err(ApiKeyAuthorizerError::AuthenticationFailed) => Ok(ApiKeyAuthorizerResponse::denied()),
        Err(ApiKeyAuthorizerError::Unavailable) => Err(ApiKeyAuthorizerError::Unavailable),
    }
}

fn credential_for_exact_route(
    request: &ApiKeyAuthorizerRequest,
) -> Result<&str, ApiKeyAuthorizerError> {
    if request.route_key.as_deref() != Some(API_KEY_ROUTE_KEY) {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    }
    let credentials = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(API_KEY_HEADER))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    let [credential] = credentials.as_slice() else {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    };
    let identity_source = request
        .identity_source
        .as_deref()
        .ok_or(ApiKeyAuthorizerError::AuthenticationFailed)?;
    let [first, second] = identity_source else {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    };
    if !((first == credential && second == API_KEY_ROUTE_KEY)
        || (first == API_KEY_ROUTE_KEY && second == credential))
    {
        return Err(ApiKeyAuthorizerError::AuthenticationFailed);
    }
    Ok(credential)
}

fn valid_owner_id(owner_id: &str) -> bool {
    owner_id
        .strip_prefix("cognito:user:")
        .is_some_and(|subject| {
            !subject.is_empty()
                && owner_id.len() <= MAX_OWNER_ID_LEN
                && !subject.chars().any(char::is_control)
                && subject.trim() == subject
        })
}

fn valid_public_id(public_id: &str) -> bool {
    public_id.len() == PUBLIC_ID_LEN
        && public_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use secrecy::ExposeSecret;
    use serde_json::json;

    use super::*;
    use crate::api_keys::{
        generate_api_key, ApiKeyPage, ApiKeyRecord, ApiKeyScope, ApiKeyStore, ApiKeyStoreError,
        CreateKeyRecord, FakeApiKeyStore, KeyEnvironment, RevokeResult, SweepPage, SweepRequest,
    };

    #[derive(Clone)]
    struct CountingStore {
        inner: FakeApiKeyStore,
        lookups: Arc<AtomicUsize>,
        fail_lookup: bool,
    }

    impl CountingStore {
        fn new(inner: FakeApiKeyStore) -> Self {
            Self {
                inner,
                lookups: Arc::new(AtomicUsize::new(0)),
                fail_lookup: false,
            }
        }

        fn failing() -> Self {
            Self {
                inner: FakeApiKeyStore::new(),
                lookups: Arc::new(AtomicUsize::new(0)),
                fail_lookup: true,
            }
        }

        fn lookup_count(&self) -> usize {
            self.lookups.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ApiKeyStore for CountingStore {
        async fn create_key(&self, request: CreateKeyRecord) -> Result<(), ApiKeyStoreError> {
            self.inner.create_key(request).await
        }

        async fn get_key_consistent(
            &self,
            public_id: &str,
        ) -> Result<Option<ApiKeyRecord>, ApiKeyStoreError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            if self.fail_lookup {
                Err(ApiKeyStoreError::Backend)
            } else {
                self.inner.get_key_consistent(public_id).await
            }
        }

        async fn list_owner_keys(
            &self,
            owner_id: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<ApiKeyPage, ApiKeyStoreError> {
            self.inner.list_owner_keys(owner_id, cursor, limit).await
        }

        async fn revoke_key(
            &self,
            owner_id: &str,
            public_id: &str,
            now: u64,
        ) -> Result<RevokeResult, ApiKeyStoreError> {
            self.inner.revoke_key(owner_id, public_id, now).await
        }

        async fn sweep_expired(
            &self,
            request: SweepRequest,
        ) -> Result<SweepPage, ApiKeyStoreError> {
            self.inner.sweep_expired(request).await
        }
    }

    fn request(credential: Option<&str>) -> ApiKeyAuthorizerRequest {
        let headers = credential.map_or_else(BTreeMap::new, |credential| {
            BTreeMap::from([("x-spur-api-key".to_owned(), credential.to_owned())])
        });
        ApiKeyAuthorizerRequest {
            route_key: Some("POST /mcp/api-key".to_owned()),
            identity_source: credential
                .map(|credential| vec![credential.to_owned(), "POST /mcp/api-key".to_owned()]),
            headers,
        }
    }

    fn generated(now: u64) -> crate::api_keys::GeneratedApiKey {
        generate_api_key(
            KeyEnvironment::Live,
            "cognito:user:authorizer-human",
            "authorizer test",
            ApiKeyScopes::new([
                ApiKeyScope::ExternalRead,
                ApiKeyScope::ExternalIndex,
                ApiKeyScope::ExternalStatus,
            ])
            .expect("scopes should be valid"),
            now,
            now + 3_600,
        )
        .expect("test key generation should succeed")
    }

    #[tokio::test]
    async fn valid_key_uses_one_lookup_and_returns_simple_v1_context() {
        let now = 1_700_000_000;
        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        let inner = FakeApiKeyStore::new();
        inner
            .create_key(CreateKeyRecord::new(generated.record))
            .await
            .expect("record should persist");
        let store = CountingStore::new(inner);

        let response = authorize_api_key(
            &request(Some(&credential)),
            &store,
            KeyEnvironment::Live,
            now,
        )
        .await
        .expect("valid key should authorize");

        assert_eq!(store.lookup_count(), 1);
        assert_eq!(
            serde_json::to_value(response).expect("simple response should serialize"),
            json!({
                "isAuthorized": true,
                "context": {
                    "auth_context_version": 1,
                    "auth_kind": "api_key",
                    "owner_id": "cognito:user:authorizer-human",
                    "key_id": generated.public_id,
                    "scopes": "external.read external.index external.status"
                }
            })
        );
    }

    #[tokio::test]
    async fn split_api_key_routes_authorize_with_exact_cache_identity() {
        let now = 1_700_000_000;
        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        let inner = FakeApiKeyStore::new();
        inner
            .create_key(CreateKeyRecord::new(generated.record))
            .await
            .expect("record should persist");
        let store = CountingStore::new(inner);

        for route in ["POST /mcp/api-key/code", "POST /mcp/api-key/knowledge"] {
            for route_first in [false, true] {
                let mut request = request(Some(&credential));
                request.route_key = Some(route.to_owned());
                let mut identity = vec![credential.clone(), route.to_owned()];
                if route_first {
                    identity.reverse();
                }
                request.identity_source = Some(identity);
                let response = authorize_api_key(&request, &store, KeyEnvironment::Live, now)
                    .await
                    .expect("deployed split route must accept an active key");
                assert!(response.is_authorized);
            }
        }
        assert_eq!(store.lookup_count(), 4);
    }

    #[tokio::test]
    async fn split_api_key_routes_reject_mismatched_identity_and_route_variants() {
        let now = 1_700_000_000;
        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        let store = CountingStore::failing();

        for route in ["POST /mcp/api-key/code", "POST /mcp/api-key/knowledge"] {
            for identity_route in [
                "POST /mcp/api-key",
                "POST /mcp/api-key/code",
                "POST /mcp/api-key/knowledge",
            ] {
                if identity_route == route {
                    continue;
                }
                let mut request = request(Some(&credential));
                request.route_key = Some(route.to_owned());
                request.identity_source = Some(vec![credential.clone(), identity_route.to_owned()]);
                assert_eq!(
                    authorize_api_key(&request, &store, KeyEnvironment::Live, now).await,
                    Err(ApiKeyAuthorizerError::AuthenticationFailed)
                );
            }
        }
        for route in [
            "GET /mcp/api-key/code",
            "POST /mcp/api-key/code/",
            "POST /mcp/api-key/knowledge/extra",
            "POST /mcp/api-key/other",
            "POST /mcp/oauth/code",
        ] {
            let mut request = request(Some(&credential));
            request.route_key = Some(route.to_owned());
            request.identity_source = Some(vec![credential.clone(), route.to_owned()]);
            assert_eq!(
                authorize_api_key(&request, &store, KeyEnvironment::Live, now).await,
                Err(ApiKeyAuthorizerError::AuthenticationFailed)
            );
        }
        assert_eq!(store.lookup_count(), 0);
    }

    #[tokio::test]
    async fn credential_failures_are_indistinguishable_and_secret_safe() {
        let now = 1_700_000_000;
        let active = generated(now);
        let other = generated(now);
        let other_secret = other
            .plaintext
            .expose_secret()
            .rsplit_once('_')
            .expect("generated key should have a secret segment")
            .1;
        let wrong_secret = format!("spur_live_{}_{}", active.public_id, other_secret);
        let unknown = generated(now);
        let unknown_credential = unknown.plaintext.expose_secret().to_owned();
        let credential = active.plaintext.expose_secret().to_owned();
        let revoked = generated(now);
        let revoked_credential = revoked.plaintext.expose_secret().to_owned();
        let revoked_owner = revoked.record.owner_id.clone();
        let revoked_id = revoked.record.public_id.clone();
        let expired = generate_api_key(
            KeyEnvironment::Live,
            "cognito:user:authorizer-human",
            "expired test",
            ApiKeyScopes::new([ApiKeyScope::ExternalRead]).expect("scope should be valid"),
            now - 7_200,
            now - 1,
        )
        .expect("expired test record should have valid historical bounds");
        let expired_credential = expired.plaintext.expose_secret().to_owned();
        let inner = FakeApiKeyStore::new();
        inner
            .create_key(CreateKeyRecord::new(active.record.clone()))
            .await
            .expect("active record should persist");
        inner
            .create_key(CreateKeyRecord::new(revoked.record))
            .await
            .expect("record to revoke should persist");
        assert_eq!(
            inner
                .revoke_key(&revoked_owner, &revoked_id, now - 1)
                .await
                .expect("record should revoke"),
            RevokeResult::Revoked
        );
        inner
            .create_key(CreateKeyRecord::new(expired.record))
            .await
            .expect("expired record should persist");
        let store = CountingStore::new(inner);

        let failures = [
            authorize_api_key(&request(None), &store, KeyEnvironment::Live, now).await,
            authorize_api_key(
                &request(Some("not-a-key")),
                &store,
                KeyEnvironment::Live,
                now,
            )
            .await,
            authorize_api_key(
                &request(Some(&unknown_credential)),
                &store,
                KeyEnvironment::Live,
                now,
            )
            .await,
            authorize_api_key(
                &request(Some(&wrong_secret)),
                &store,
                KeyEnvironment::Live,
                now,
            )
            .await,
            authorize_api_key(
                &request(Some(&revoked_credential)),
                &store,
                KeyEnvironment::Live,
                now,
            )
            .await,
            authorize_api_key(
                &request(Some(&expired_credential)),
                &store,
                KeyEnvironment::Live,
                now,
            )
            .await,
        ];
        for failure in failures {
            let error = failure.expect_err("credential must fail");
            assert_eq!(error.status_code(), 401);
            assert_eq!(
                error.body(),
                r#"{"error":{"code":"authentication_failed"}}"#
            );
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(&credential));
            assert!(!rendered.contains(&unknown_credential));
            assert!(!rendered.contains(&format!("{:?}", active.record.secret_hash)));
        }
    }

    #[tokio::test]
    async fn authorizer_requires_exact_route_identity_and_fails_closed_on_store_error() {
        let now = 1_700_000_000;
        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        let store = CountingStore::failing();

        let error = authorize_api_key(
            &request(Some(&credential)),
            &store,
            KeyEnvironment::Live,
            now,
        )
        .await
        .expect_err("store error must fail closed");
        assert_eq!(store.lookup_count(), 1);
        assert_eq!(error.status_code(), 503);
        assert_eq!(
            error.body(),
            r#"{"error":{"code":"authorizer_unavailable"}}"#
        );

        let mut wrong_route = request(Some(&credential));
        wrong_route.route_key = Some("POST /mcp/oauth".to_owned());
        let error = authorize_api_key(&wrong_route, &store, KeyEnvironment::Live, now)
            .await
            .expect_err("wrong route must not use the store");
        assert_eq!(error.status_code(), 401);
        assert_eq!(store.lookup_count(), 1);
    }

    #[tokio::test]
    async fn identity_source_accepts_provider_order_and_rejects_non_exact_shapes_without_lookup() {
        let now = 1_700_000_000;
        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        let inner = FakeApiKeyStore::new();
        inner
            .create_key(CreateKeyRecord::new(generated.record))
            .await
            .expect("record should persist");
        let store = CountingStore::new(inner);

        let mut route_first = request(Some(&credential));
        route_first.identity_source =
            Some(vec!["POST /mcp/api-key".to_owned(), credential.clone()]);
        let response = authorize_api_key(&route_first, &store, KeyEnvironment::Live, now)
            .await
            .expect("provider-normalized route-first identity must authorize");
        assert!(response.is_authorized);
        assert_eq!(store.lookup_count(), 1);

        let malformed_identities = [
            None,
            Some(vec![credential.clone()]),
            Some(vec![credential.clone(), credential.clone()]),
            Some(vec![
                "POST /mcp/api-key".to_owned(),
                "POST /mcp/api-key".to_owned(),
            ]),
            Some(vec![
                credential.clone(),
                "POST /mcp/api-key".to_owned(),
                "extra".to_owned(),
            ]),
            Some(vec![
                "POST /mcp/api-key".to_owned(),
                "mismatched-credential".to_owned(),
            ]),
            Some(vec![credential.clone(), "POST /mcp/oauth".to_owned()]),
        ];
        for identity_source in malformed_identities {
            let mut malformed = request(Some(&credential));
            malformed.identity_source = identity_source;
            assert_eq!(
                authorize_api_key(&malformed, &store, KeyEnvironment::Live, now).await,
                Err(ApiKeyAuthorizerError::AuthenticationFailed)
            );
            assert_eq!(store.lookup_count(), 1);
        }

        let mut duplicate_header = request(Some(&credential));
        duplicate_header
            .headers
            .insert("X-SPUR-API-Key".to_owned(), credential.clone());
        assert_eq!(
            authorize_api_key(&duplicate_header, &store, KeyEnvironment::Live, now).await,
            Err(ApiKeyAuthorizerError::AuthenticationFailed)
        );
        assert_eq!(store.lookup_count(), 1);

        let mut missing_header = request(Some(&credential));
        missing_header.headers.clear();
        assert_eq!(
            authorize_api_key(&missing_header, &store, KeyEnvironment::Live, now).await,
            Err(ApiKeyAuthorizerError::AuthenticationFailed)
        );
        assert_eq!(store.lookup_count(), 1);
    }

    #[tokio::test]
    async fn boundary_accepts_only_the_configured_live_or_test_environment() {
        let now = 1_700_000_000;
        for environment in [KeyEnvironment::Live, KeyEnvironment::Test] {
            let generated = generate_api_key(
                environment,
                "cognito:user:environment-human",
                "environment test",
                ApiKeyScopes::new([ApiKeyScope::ExternalRead]).expect("scope should be valid"),
                now,
                now + 3_600,
            )
            .expect("test key generation should succeed");
            let credential = generated.plaintext.expose_secret().to_owned();
            let inner = FakeApiKeyStore::new();
            inner
                .create_key(CreateKeyRecord::new(generated.record))
                .await
                .expect("record should persist");
            let store = CountingStore::new(inner);

            let response = authorize_api_key_with_environment(
                &request(Some(&credential)),
                &store,
                Some(environment.as_str()),
                now,
            )
            .await
            .expect("matching environment should authorize");

            assert_eq!(store.lookup_count(), 1);
            assert_eq!(
                serde_json::to_value(response).expect("response should serialize")["isAuthorized"],
                true
            );
        }

        let test_key = generate_api_key(
            KeyEnvironment::Test,
            "cognito:user:environment-human",
            "cross environment test",
            ApiKeyScopes::new([ApiKeyScope::ExternalRead]).expect("scope should be valid"),
            now,
            now + 3_600,
        )
        .expect("test key generation should succeed");
        let credential = test_key.plaintext.expose_secret().to_owned();
        let store = CountingStore::new(FakeApiKeyStore::new());
        let response = authorize_api_key_with_environment(
            &request(Some(&credential)),
            &store,
            Some("live"),
            now,
        )
        .await
        .expect("credential denial should be a cacheable simple response");
        assert_eq!(store.lookup_count(), 0);
        assert_eq!(
            serde_json::to_value(response).expect("response should serialize")["isAuthorized"],
            false
        );
    }

    #[tokio::test]
    async fn boundary_defaults_live_and_rejects_bad_environment_without_lookup() {
        let now = 1_700_000_000;
        let live = generated(now);
        let credential = live.plaintext.expose_secret().to_owned();
        let inner = FakeApiKeyStore::new();
        inner
            .create_key(CreateKeyRecord::new(live.record))
            .await
            .expect("record should persist");
        let store = CountingStore::new(inner);

        let response =
            authorize_api_key_with_environment(&request(Some(&credential)), &store, None, now)
                .await
                .expect("missing setting should default to live");
        assert_eq!(
            serde_json::to_value(response).expect("response should serialize")["isAuthorized"],
            true
        );

        let lookups_before = store.lookup_count();
        assert_eq!(
            authorize_api_key_with_environment(
                &request(Some(&credential)),
                &store,
                Some("staging"),
                now,
            )
            .await,
            Err(ApiKeyAuthorizerError::Unavailable)
        );
        assert_eq!(store.lookup_count(), lookups_before);
    }

    #[tokio::test]
    async fn boundary_serializes_cacheable_denies_but_propagates_store_failures() {
        let now = 1_700_000_000;
        let denied = authorize_api_key_with_environment(
            &request(Some("not-a-key")),
            &CountingStore::new(FakeApiKeyStore::new()),
            Some("live"),
            now,
        )
        .await
        .expect("credential denial should not become a Lambda failure");
        assert_eq!(
            serde_json::to_value(denied).expect("deny response should serialize"),
            json!({
                "isAuthorized": false,
                "context": {
                    "auth_context_version": 1,
                    "auth_kind": "api_key",
                    "denial_code": "authentication_failed"
                }
            })
        );

        let generated = generated(now);
        let credential = generated.plaintext.expose_secret().to_owned();
        assert_eq!(
            authorize_api_key_with_environment(
                &request(Some(&credential)),
                &CountingStore::failing(),
                Some("live"),
                now,
            )
            .await,
            Err(ApiKeyAuthorizerError::Unavailable)
        );
    }

    #[test]
    fn dedicated_authorizer_binary_keeps_the_catalog_and_service_library_out() {
        let source = include_str!("bin/api_key_authorizer.rs");

        assert!(source.contains("#[path = \"../api_keys.rs\"]"));
        assert!(source.contains("#[path = \"../api_key_authorizer.rs\"]"));
        for forbidden in [
            "spur_context_service",
            "duckdb",
            "mod catalog",
            "crate::catalog",
            "mod mcp",
            "crate::mcp",
        ] {
            assert!(
                !source.contains(forbidden),
                "authorizer binary must not link {forbidden}"
            );
        }
    }

    #[test]
    fn dedicated_authorizer_manifest_can_disable_the_service_dependency_graph() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("api-key-authorizer = [\"dep:lambda_runtime\"]"));
        assert!(manifest.contains("required-features = [\"api-key-authorizer\"]"));
        assert!(manifest.contains(
            "duckdb = { version = \"=1.10504.0\", features = [\"bundled\"], optional = true }"
        ));
    }
}
