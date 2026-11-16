//! DuckLake-backed code context service for external packages.

#[cfg(any(feature = "service", feature = "code-control"))]
pub mod abuse;
pub mod api_key_authorizer;
pub mod api_key_cleanup;
pub mod api_keys;
pub mod artifact_cache;
#[cfg(any(feature = "service", feature = "lambda-http"))]
mod auth;
#[cfg(feature = "service")]
pub mod catalog;
#[cfg(feature = "code-backend")]
pub mod code_backend;
#[cfg(any(feature = "service", feature = "code-control"))]
pub mod drainer;
#[cfg(any(feature = "service", feature = "code-control"))]
pub mod jobs;
#[cfg(feature = "service")]
pub mod knowledge;
#[cfg(any(feature = "code-lambda", feature = "knowledge-lambda"))]
pub mod lambda;
#[cfg(feature = "lambda-http")]
mod lambda_http;
#[cfg(feature = "service")]
pub mod mcp;
#[cfg(all(feature = "code-control", not(feature = "service")))]
pub(crate) use crate::lambda::code_control as mcp;
pub mod medallion;
#[cfg(any(feature = "service", feature = "code-backend"))]
mod package_search;
#[cfg(feature = "service")]
pub mod query;
pub mod serving_registry;
#[cfg(feature = "service")]
pub mod staleness;
#[cfg(feature = "service")]
pub mod translate;
#[cfg(feature = "worker")]
pub mod worker;

#[cfg(all(test, feature = "code-lambda"))]
mod serving_lambda_contract {
    use crate::auth::RequestRoute;
    use crate::lambda::{control_route_is_eligible, tool_is_eligible, BackendKind};

    const CODE_TOOLS: [&str; 7] = [
        "external_catalog",
        "external_index",
        "external_index_status",
        "external_code_search",
        "external_code_read",
        "external_code_callers",
        "external_code_callees",
    ];
    const KNOWLEDGE_TOOL: &str = "external_knowledge_context";

    #[test]
    fn code_lambda_tool_eligibility() {
        assert!(cfg!(feature = "code-lambda"));
        assert!(
            !cfg!(feature = "service"),
            "the Code Lambda feature must not enable the DuckDB-bearing service closure"
        );

        for (backend, tool, expected) in CODE_TOOLS
            .into_iter()
            .map(|tool| (BackendKind::Code, tool, true))
            .chain([(BackendKind::Code, KNOWLEDGE_TOOL, false)])
            .chain(
                CODE_TOOLS
                    .into_iter()
                    .map(|tool| (BackendKind::Knowledge, tool, false)),
            )
            .chain([(BackendKind::Knowledge, KNOWLEDGE_TOOL, true)])
        {
            assert_eq!(
                tool_is_eligible(backend, tool),
                expected,
                "wrong eligibility for {backend:?}/{tool}"
            );
        }
    }

    #[test]
    fn code_owns_control_routes_and_knowledge_rejects_them() {
        for route in [
            RequestRoute::Discovery,
            RequestRoute::Login,
            RequestRoute::ApiKeyCreate,
            RequestRoute::ApiKeyList,
            RequestRoute::ApiKeyRevoke,
        ] {
            assert!(
                control_route_is_eligible(BackendKind::Code, route),
                "Code must own {route:?}"
            );
            assert!(
                !control_route_is_eligible(BackendKind::Knowledge, route),
                "Knowledge must reject {route:?}"
            );
        }
    }
}

#[cfg(all(test, feature = "lambda-http"))]
mod lambda_http_contract {
    use serde_json::json;

    use crate::auth::{AuthFailure, RequestRoute};
    use crate::lambda_http::{
        authenticated_caller_id, classify_route, reject_api_key_auth_on_wrong_route,
        reject_jwt_auth_on_wrong_route, ApiGatewayRequest,
    };

    fn request(path: &str, method: &str) -> ApiGatewayRequest {
        serde_json::from_value(json!({
            "rawPath": path,
            "requestContext": { "http": { "method": method } }
        }))
        .expect("API Gateway request should deserialize")
    }

    fn jwt_request(path: &str, method: &str) -> ApiGatewayRequest {
        serde_json::from_value(json!({
            "rawPath": path,
            "requestContext": {
                "http": { "method": method },
                "authorizer": { "jwt": { "claims": { "sub": "caller" } } }
            }
        }))
        .expect("JWT-authorized API Gateway request should deserialize")
    }

    fn api_key_request(path: &str, method: &str) -> ApiGatewayRequest {
        serde_json::from_value(json!({
            "rawPath": path,
            "requestContext": {
                "http": { "method": method },
                "authorizer": { "lambda": { "principalId": "key-owner" } }
            }
        }))
        .expect("API-key-authorized API Gateway request should deserialize")
    }

    #[test]
    fn oauth_path_classifies_through_lambda_http() {
        assert_eq!(
            classify_route(&request("/mcp/oauth", "POST")),
            RequestRoute::OAuth
        );
    }

    #[test]
    fn api_key_path_classifies_through_lambda_http() {
        assert_eq!(
            classify_route(&request("/mcp/api-key", "POST")),
            RequestRoute::ApiKeyMcp
        );
    }

    #[test]
    fn management_paths_classify_through_lambda_http() {
        assert_eq!(
            classify_route(&request("/auth/api-keys", "POST")),
            RequestRoute::ApiKeyCreate
        );
        assert_eq!(
            classify_route(&request("/auth/api-keys", "GET")),
            RequestRoute::ApiKeyList
        );
        assert_eq!(
            classify_route(&request("/auth/api-keys/key-123", "DELETE")),
            RequestRoute::ApiKeyRevoke
        );
    }

    #[test]
    fn direct_tool_paths_keep_legacy_route_classification() {
        for path in ["/mcp/index", "/mcp/index_status"] {
            assert_eq!(classify_route(&request(path, "POST")), RequestRoute::Legacy);
        }
    }

    #[test]
    fn direct_unauthenticated_paths_keep_legacy_route_classification() {
        for path in ["/mcp", "/mcp/code", "/mcp/knowledge"] {
            assert_eq!(classify_route(&request(path, "POST")), RequestRoute::Legacy);
        }
    }

    #[test]
    fn jwt_route_guard_accepts_exact_oauth_splits_and_rejects_other_routes() {
        for path in ["/mcp/oauth/code", "/mcp/oauth/knowledge"] {
            assert_eq!(
                reject_jwt_auth_on_wrong_route(&jwt_request(path, "POST")),
                Ok(())
            );
        }

        for request in [
            jwt_request("/mcp/api-key/code", "POST"),
            jwt_request("/mcp/code", "POST"),
            jwt_request("/mcp/oauth/code", "GET"),
            jwt_request("/mcp/oauth/code/extra", "POST"),
        ] {
            assert_eq!(
                reject_jwt_auth_on_wrong_route(&request),
                Err(AuthFailure::WrongRoute)
            );
        }
    }

    #[test]
    fn api_key_route_guard_accepts_exact_api_key_splits_and_rejects_other_routes() {
        for path in ["/mcp/api-key/code", "/mcp/api-key/knowledge"] {
            assert_eq!(
                reject_api_key_auth_on_wrong_route(&api_key_request(path, "POST")),
                Ok(())
            );
        }

        for request in [
            api_key_request("/mcp/oauth/code", "POST"),
            api_key_request("/mcp/code", "POST"),
            api_key_request("/mcp/api-key/code", "GET"),
            api_key_request("/mcp/api-key/code/extra", "POST"),
        ] {
            assert_eq!(
                reject_api_key_auth_on_wrong_route(&request),
                Err(AuthFailure::WrongRoute)
            );
        }
    }

    #[test]
    fn unauthenticated_mutating_caller_error_keeps_legacy_message() {
        assert_eq!(
            authenticated_caller_id(&request("/mcp", "POST"), false)
                .expect_err("unauthenticated mutation should be rejected")
                .to_string(),
            "authenticated caller is required for mutating context-service tools"
        );
    }
}
