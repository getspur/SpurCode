variable "aws_region" {
  description = "AWS region for all resources"
  type        = string
  default     = "ap-southeast-5"
}

variable "bucket_name" {
  description = "S3 bucket for DuckLake catalog and Parquet data"
  type        = string
  default     = "spur-context"
}

variable "lambda_zip_path" {
  description = "Legacy serving Lambda zip path used only while both distinct serving ZIP overrides are omitted"
  type        = string
  default     = "../../target/lambda/spur-context-service.zip"
}

variable "code_lambda_zip_path" {
  description = "Optional local path to the DuckDB-free Code Lambda zip; Task 14 supplies it together with knowledge_lambda_zip_path"
  type        = string
  default     = null
  nullable    = true

  validation {
    condition     = var.code_lambda_zip_path == null ? true : length(trimspace(var.code_lambda_zip_path)) > 0
    error_message = "code_lambda_zip_path must be null or a non-empty path."
  }
}

variable "knowledge_lambda_zip_path" {
  description = "Optional local path to the extension-bearing Knowledge Lambda zip; Task 14 supplies it together with code_lambda_zip_path"
  type        = string
  default     = null
  nullable    = true

  validation {
    condition     = var.knowledge_lambda_zip_path == null ? true : length(trimspace(var.knowledge_lambda_zip_path)) > 0
    error_message = "knowledge_lambda_zip_path must be null or a non-empty path."
  }
}

variable "catalog_s3_uri" {
  description = "S3 URI of the frozen serving DuckLake snapshot pointer or snapshot file"
  type        = string
  default     = "s3://spur-context/gold/catalog-snapshot/current.json"
}

variable "allow_anonymous_mutations" {
  description = "Allow mutating tools (external_index/external_index_status) without an authenticated caller, falling back to a shared anonymous identity. Intended for internal-team / trusted-network stacks where the API route is public (NONE). Secure-by-default off; the shared anonymous identity still shares the per-caller rate limit / active-job cap."
  type        = bool
  default     = false
}

variable "api_authorization_type" {
  description = "Authorization for the direct POST /mcp/code and POST /mcp/knowledge compatibility routes. AWS_IAM (SigV4, secure default) or NONE (public, unauthenticated — use only for demo/eval stacks). Valid: NONE, AWS_IAM, JWT, CUSTOM."
  type        = string
  default     = "AWS_IAM"

  validation {
    condition     = contains(["NONE", "AWS_IAM", "JWT", "CUSTOM"], var.api_authorization_type)
    error_message = "api_authorization_type must be one of NONE, AWS_IAM, JWT, CUSTOM."
  }
}

# ─── Additive Cognito OAuth ingress ─────────────────────────────────────────
# These values are intentionally metadata only. Generated M2M app-client
# secrets remain provider-state data and must never be supplied to Lambda,
# committed to tfvars, or exposed through an output.

variable "cognito_auth_enabled" {
  description = "Enable the additive Cognito Lite user pool, JWT authorizer, and exact POST /mcp/oauth/code and POST /mcp/oauth/knowledge routes. Disabled by default so the direct compatibility-route behavior is unchanged."
  type        = bool
  default     = false
}

# ─── Stable public domains and migration controls ───────────────────────────
# The hosted zone is always bootstrapped so its nameservers can be delegated
# before ACM validation resources exist. Activation and execute-api retirement
# are deliberately independent operator decisions.

variable "custom_domains_enabled" {
  description = "Activate DNS-validated certificates and the context.getspur.dev API plus auth.context.getspur.dev Cognito custom domains. The delegated hosted zone is created regardless."
  type        = bool
  default     = false
}

variable "disable_execute_api_endpoint" {
  description = "Disable the API Gateway execute-api endpoint after clients have migrated to context.getspur.dev. Requires custom_domains_enabled."
  type        = bool
  default     = false
}

# ─── Additive personal API-key ingress ───────────────────────────────────────
# API-key resources are independent of the direct compatibility and OAuth routes and
# are omitted entirely unless this feature is explicitly enabled.

variable "api_key_auth_enabled" {
  description = "Enable personal API-key discovery, management, authorizer, MCP, storage, cleanup, logs, and alarms. Requires Cognito human authentication and is disabled by default."
  type        = bool
  default     = false

  validation {
    condition     = !var.api_key_auth_enabled || var.cognito_auth_enabled
    error_message = "api_key_auth_enabled requires cognito_auth_enabled so only Cognito humans can manage personal keys."
  }

  validation {
    condition = (
      !var.api_key_auth_enabled ||
      contains(var.cognito_human_callback_urls, "http://127.0.0.1:8765/callback")
    )
    error_message = "api_key_auth_enabled requires the exact CLI callback http://127.0.0.1:8765/callback in cognito_human_callback_urls."
  }
}

variable "api_key_table_name" {
  description = "Dedicated DynamoDB table for personal API-key records, owner counters, and the persisted cleanup cursor."
  type        = string
  default     = "spur-context-api-keys"

  validation {
    condition     = can(regex("^[A-Za-z0-9_.-]{3,255}$", var.api_key_table_name))
    error_message = "api_key_table_name must be a valid 3-255 character DynamoDB table name."
  }
}

variable "api_key_owner_gsi_name" {
  description = "Owner GSI used for personal-key listing and bounded revoke-by-owner operations. Fixed to the backend contract in v1."
  type        = string
  default     = "owner-gsi"

  validation {
    condition     = var.api_key_owner_gsi_name == "owner-gsi"
    error_message = "api_key_owner_gsi_name must be owner-gsi in v1."
  }
}

variable "api_key_expiry_gsi_name" {
  description = "Sparse UTC expiry-hour GSI used by the bounded cleanup worker. Fixed to the backend contract in v1."
  type        = string
  default     = "expiry-gsi"

  validation {
    condition     = var.api_key_expiry_gsi_name == "expiry-gsi"
    error_message = "api_key_expiry_gsi_name must be expiry-gsi in v1."
  }
}

variable "api_key_authorizer_zip_path" {
  description = "Independent lean Lambda zip produced by scripts/package-context-api-key-lambdas.sh for the API-key authorizer bootstrap."
  type        = string
  default     = "../../target/lambda/spur-context-api-key-authorizer.zip"

  validation {
    condition     = !var.api_key_auth_enabled || var.api_key_authorizer_zip_path != var.lambda_zip_path
    error_message = "api_key_authorizer_zip_path must be independent from the serving lambda_zip_path."
  }
}

variable "api_key_cleanup_zip_path" {
  description = "Independent lean Lambda zip produced by scripts/package-context-api-key-lambdas.sh for bounded API-key expiry cleanup. It must not reuse the serving or authorizer artifact."
  type        = string
  default     = "../../target/lambda/spur-context-api-key-cleanup.zip"

  validation {
    condition = !var.api_key_auth_enabled || (
      var.api_key_cleanup_zip_path != var.lambda_zip_path &&
      var.api_key_cleanup_zip_path != var.api_key_authorizer_zip_path
    )
    error_message = "api_key_cleanup_zip_path must be independent from the serving and authorizer artifacts."
  }
}

variable "api_key_environment" {
  description = "Key prefix environment accepted by production infrastructure. Use test only in isolated POC stacks."
  type        = string
  default     = "live"

  validation {
    condition     = contains(["live", "test"], var.api_key_environment)
    error_message = "api_key_environment must be live or test."
  }
}

variable "api_key_authorizer_cache_seconds" {
  description = "API Gateway request-authorizer result TTL. V1 fixes this at 30 seconds to bound revocation delay."
  type        = number
  default     = 30

  validation {
    condition     = var.api_key_authorizer_cache_seconds == 30
    error_message = "api_key_authorizer_cache_seconds must be exactly 30 seconds in v1."
  }
}

variable "api_key_default_ttl_days" {
  description = "Default personal API-key lifetime in whole days."
  type        = number
  default     = 90

  validation {
    condition     = var.api_key_default_ttl_days >= 1 && var.api_key_default_ttl_days <= var.api_key_max_ttl_days && floor(var.api_key_default_ttl_days) == var.api_key_default_ttl_days
    error_message = "api_key_default_ttl_days must be a whole number from 1 through api_key_max_ttl_days."
  }
}

variable "api_key_max_ttl_days" {
  description = "Maximum personal API-key lifetime in whole days. V1 caps this at 365."
  type        = number
  default     = 365

  validation {
    condition     = var.api_key_max_ttl_days >= 1 && var.api_key_max_ttl_days <= 365 && floor(var.api_key_max_ttl_days) == var.api_key_max_ttl_days
    error_message = "api_key_max_ttl_days must be a whole number from 1 through 365."
  }
}

variable "api_key_max_active_per_user" {
  description = "Maximum active personal keys per Cognito human. V1 fixes this at ten."
  type        = number
  default     = 10

  validation {
    condition     = var.api_key_max_active_per_user == 10
    error_message = "api_key_max_active_per_user must be exactly 10 in v1."
  }
}

variable "api_key_authorizer_memory_mb" {
  description = "Memory allocation for the lean API-key authorizer Lambda."
  type        = number
  default     = 256

  validation {
    condition     = var.api_key_authorizer_memory_mb >= 128 && var.api_key_authorizer_memory_mb <= 1024
    error_message = "api_key_authorizer_memory_mb must be between 128 and 1024 MB."
  }
}

variable "api_key_authorizer_timeout_sec" {
  description = "Timeout for the lean API-key authorizer Lambda."
  type        = number
  default     = 5

  validation {
    condition     = var.api_key_authorizer_timeout_sec >= 1 && var.api_key_authorizer_timeout_sec <= 30
    error_message = "api_key_authorizer_timeout_sec must be between 1 and 30 seconds."
  }
}

variable "api_key_authorizer_log_retention_days" {
  description = "CloudWatch retention for the lean API-key authorizer logs."
  type        = number
  default     = 14
}

variable "api_key_cleanup_memory_mb" {
  description = "Memory allocation for the bounded API-key expiry cleanup Lambda."
  type        = number
  default     = 256

  validation {
    condition     = var.api_key_cleanup_memory_mb >= 128 && var.api_key_cleanup_memory_mb <= 1024
    error_message = "api_key_cleanup_memory_mb must be between 128 and 1024 MB."
  }
}

variable "api_key_cleanup_timeout_sec" {
  description = "Timeout for one bounded API-key expiry cleanup invocation."
  type        = number
  default     = 60

  validation {
    condition     = var.api_key_cleanup_timeout_sec >= 1 && var.api_key_cleanup_timeout_sec <= 300
    error_message = "api_key_cleanup_timeout_sec must be between 1 and 300 seconds."
  }
}

variable "api_key_cleanup_log_retention_days" {
  description = "CloudWatch retention for API-key expiry cleanup logs."
  type        = number
  default     = 14
}

variable "api_key_cleanup_max_catchup_hours" {
  description = "Historical expiry-hour horizon used only when the persisted cleanup cursor is absent or behind. Per-invocation work is bounded separately."
  type        = number
  default     = 168

  validation {
    condition     = var.api_key_cleanup_max_catchup_hours >= 1 && var.api_key_cleanup_max_catchup_hours <= 8760 && floor(var.api_key_cleanup_max_catchup_hours) == var.api_key_cleanup_max_catchup_hours
    error_message = "api_key_cleanup_max_catchup_hours must be a whole number between 1 and 8760."
  }
}

variable "api_key_cleanup_schedule_minutes" {
  description = "EventBridge cadence for lease-fenced API-key cleanup. At most 15 minutes preserves supported steady-state capacity with the bounded record limit."
  type        = number
  default     = 5

  validation {
    condition     = var.api_key_cleanup_schedule_minutes >= 2 && var.api_key_cleanup_schedule_minutes <= 15 && floor(var.api_key_cleanup_schedule_minutes) == var.api_key_cleanup_schedule_minutes
    error_message = "api_key_cleanup_schedule_minutes must be a whole number between 2 and 15."
  }
}

variable "api_key_cleanup_max_buckets" {
  description = "Maximum forward expiry-hour buckets completed by one cleanup invocation."
  type        = number
  default     = 4

  validation {
    condition     = var.api_key_cleanup_max_buckets >= 1 && var.api_key_cleanup_max_buckets <= 8 && floor(var.api_key_cleanup_max_buckets) == var.api_key_cleanup_max_buckets
    error_message = "api_key_cleanup_max_buckets must be a whole number between 1 and 8."
  }
}

variable "api_key_cleanup_max_pages" {
  description = "Maximum expiry-GSI query pages, including late-index overlap pages, attempted by one cleanup invocation."
  type        = number
  default     = 8

  validation {
    condition     = var.api_key_cleanup_max_pages >= var.api_key_cleanup_max_buckets + 2 && var.api_key_cleanup_max_pages <= 16 && floor(var.api_key_cleanup_max_pages) == var.api_key_cleanup_max_pages
    error_message = "api_key_cleanup_max_pages must be a whole number no smaller than max_buckets + 2 and no greater than 16."
  }
}

variable "api_key_cleanup_max_records" {
  description = "Maximum expiry records attempted by one cleanup invocation across all forward and overlap pages."
  type        = number
  default     = 100

  validation {
    condition     = var.api_key_cleanup_max_records >= 1 && var.api_key_cleanup_max_records <= 100 && floor(var.api_key_cleanup_max_records) == var.api_key_cleanup_max_records
    error_message = "api_key_cleanup_max_records must be a whole number between 1 and 100."
  }
}

variable "api_key_cleanup_page_limit" {
  description = "Maximum expiry-GSI records attempted in one query page; the per-invocation record bound applies across pages."
  type        = number
  default     = 100

  validation {
    condition     = var.api_key_cleanup_page_limit >= 1 && var.api_key_cleanup_page_limit <= 100 && floor(var.api_key_cleanup_page_limit) == var.api_key_cleanup_page_limit
    error_message = "api_key_cleanup_page_limit must be a whole number between 1 and the backend maximum of 100."
  }
}

variable "api_key_cleanup_cursor_lag_alarm_hours" {
  description = "Cleanup cursor lag in hours that triggers the API-key cleanup alarm."
  type        = number
  default     = 2

  validation {
    condition     = var.api_key_cleanup_cursor_lag_alarm_hours >= 1
    error_message = "api_key_cleanup_cursor_lag_alarm_hours must be at least one hour."
  }
}

variable "api_key_alarm_action_arns" {
  description = "Optional SNS topic ARNs notified by API-key route, authorizer, and cleanup alarms."
  type        = set(string)
  default     = []
}

variable "cognito_user_pool_name" {
  description = "Environment-qualified Cognito user-pool name. Required when cognito_auth_enabled is true."
  type        = string
  default     = null

  validation {
    condition = !var.cognito_auth_enabled || (
      var.cognito_user_pool_name != null &&
      can(regex("^[A-Za-z0-9][A-Za-z0-9 _+=,.@-]{0,127}$", var.cognito_user_pool_name))
    )
    error_message = "cognito_user_pool_name must be a nonblank Cognito-compatible, environment-qualified name when Cognito auth is enabled."
  }
}

variable "cognito_domain_prefix" {
  description = "Unique lowercase Cognito hosted-domain prefix. Required when cognito_auth_enabled is true; use a non-production placeholder in committed examples."
  type        = string
  default     = null

  validation {
    condition = !var.cognito_auth_enabled || (
      var.cognito_domain_prefix != null &&
      can(regex("^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$", var.cognito_domain_prefix))
    )
    error_message = "cognito_domain_prefix must be a 1-63 character lowercase Cognito-compatible domain prefix when Cognito auth is enabled."
  }
}

variable "cognito_user_pool_deletion_protection" {
  description = "Keep Cognito deletion protection active except for an explicitly disposable isolated POC."
  type        = bool
  default     = true
}

variable "cognito_resource_server_identifier" {
  description = "Stable resource-server identifier used to construct Cognito access-token custom scopes. Changing it after launch breaks tokens and Lambda authorization."
  type        = string
  default     = "urn:spur:context-service"

  validation {
    condition     = length(trimspace(var.cognito_resource_server_identifier)) > 0 && length(var.cognito_resource_server_identifier) <= 256
    error_message = "cognito_resource_server_identifier must be a nonblank value of at most 256 characters."
  }
}

variable "cognito_human_callback_urls" {
  description = "Exact human OAuth callback URLs. Required when enabled; URLs must be HTTPS except localhost/127.0.0.1/[::1] loopback POC callbacks, and wildcards are forbidden."
  type        = set(string)
  default     = []

  validation {
    condition = !var.cognito_auth_enabled || (
      length(var.cognito_human_callback_urls) > 0 &&
      alltrue([
        for url in var.cognito_human_callback_urls :
        !strcontains(url, "*") && (
          can(regex("^https://[^/?#\\s@]+(?::[0-9]{1,5})?(?:/[^?#\\s]*)?(?:\\?[^#\\s]*)?$", url)) ||
          can(regex("^http://(localhost|127\\.0\\.0\\.1|\\[::1\\])(?::[0-9]{1,5})?(?:/[^?#\\s]*)?(?:\\?[^#\\s]*)?$", url))
        )
      ])
    )
    error_message = "cognito_human_callback_urls must be nonempty when enabled and contain exact HTTPS URLs or loopback-only HTTP POC URLs without wildcards."
  }
}

variable "cognito_human_logout_urls" {
  description = "Exact human OAuth logout URLs. Required when enabled; URLs must be HTTPS except localhost/127.0.0.1/[::1] loopback POC URLs, and wildcards are forbidden."
  type        = set(string)
  default     = []

  validation {
    condition = !var.cognito_auth_enabled || (
      length(var.cognito_human_logout_urls) > 0 &&
      alltrue([
        for url in var.cognito_human_logout_urls :
        !strcontains(url, "*") && (
          can(regex("^https://[^/?#\\s@]+(?::[0-9]{1,5})?(?:/[^?#\\s]*)?(?:\\?[^#\\s]*)?$", url)) ||
          can(regex("^http://(localhost|127\\.0\\.0\\.1|\\[::1\\])(?::[0-9]{1,5})?(?:/[^?#\\s]*)?(?:\\?[^#\\s]*)?$", url))
        )
      ])
    )
    error_message = "cognito_human_logout_urls must be nonempty when enabled and contain exact HTTPS URLs or loopback-only HTTP POC URLs without wildcards."
  }
}

variable "cognito_human_oidc_scopes" {
  description = "OIDC scopes for the public human client. Keep profile/email opt-in; openid is required for the authorization-code login flow."
  type        = set(string)
  default     = ["openid"]

  validation {
    condition = contains(var.cognito_human_oidc_scopes, "openid") && alltrue([
      for scope in var.cognito_human_oidc_scopes :
      contains(["openid", "profile", "email"], scope)
    ])
    error_message = "cognito_human_oidc_scopes must include openid and may contain only openid, profile, and email."
  }
}

variable "google_oauth_enabled" {
  description = "Enable Google as a social identity provider for the Cognito human app client. Google remains disabled by default and requires Cognito auth plus protected client credentials."
  type        = bool
  default     = false

  validation {
    condition     = !var.google_oauth_enabled || var.cognito_auth_enabled
    error_message = "google_oauth_enabled requires cognito_auth_enabled."
  }
}

variable "google_oauth_client_id" {
  description = "Google Auth Platform web client ID. Supply through a protected TF_VAR value; never commit it to tfvars."
  type        = string
  default     = ""
  sensitive   = true

  validation {
    condition = !var.google_oauth_enabled || can(regex(
      "^[0-9]+-[A-Za-z0-9_-]+\\.apps\\.googleusercontent\\.com$",
      trimspace(var.google_oauth_client_id),
    ))
    error_message = "google_oauth_client_id must be a Google web client ID ending in .apps.googleusercontent.com when Google OAuth is enabled."
  }
}

variable "google_oauth_client_secret" {
  description = "Google Auth Platform web client secret. Supply through a protected TF_VAR value; Terraform state contains this sensitive value."
  type        = string
  default     = ""
  sensitive   = true

  validation {
    condition     = !var.google_oauth_enabled || (length(trimspace(var.google_oauth_client_secret)) > 0 && length(var.google_oauth_client_secret) <= 2048)
    error_message = "google_oauth_client_secret must be nonblank and at most 2048 characters when Google OAuth is enabled."
  }
}

variable "cognito_human_custom_scopes" {
  description = "Least-privilege custom-scope suffixes for the human public client."
  type        = set(string)
  default     = ["external.read", "external.index", "external.status"]

  validation {
    condition = alltrue([
      for scope in var.cognito_human_custom_scopes :
      contains(["external.read", "external.index", "external.status"], scope)
    ])
    error_message = "cognito_human_custom_scopes may contain only external.read, external.index, and external.status."
  }
}

variable "cognito_human_access_token_minutes" {
  description = "Human access and ID token lifetime in whole minutes. The balanced production recommendation is 60 minutes."
  type        = number
  default     = 60

  validation {
    condition     = var.cognito_human_access_token_minutes >= 5 && var.cognito_human_access_token_minutes <= 1440 && floor(var.cognito_human_access_token_minutes) == var.cognito_human_access_token_minutes
    error_message = "cognito_human_access_token_minutes must be a whole number between 5 and 1440."
  }
}

variable "cognito_human_refresh_token_days" {
  description = "Human refresh-token lifetime in whole days. Align this value with the product session policy."
  type        = number
  default     = 30

  validation {
    condition     = var.cognito_human_refresh_token_days >= 1 && var.cognito_human_refresh_token_days <= 3650 && floor(var.cognito_human_refresh_token_days) == var.cognito_human_refresh_token_days
    error_message = "cognito_human_refresh_token_days must be a whole number between 1 and 3650."
  }
}

variable "cognito_m2m_default_access_token_hours" {
  description = "Default M2M access-token lifetime in whole hours. Six hours is the balanced default; 24-hour enabled clients require recorded risk metadata."
  type        = number
  default     = 6

  validation {
    condition     = var.cognito_m2m_default_access_token_hours >= 1 && var.cognito_m2m_default_access_token_hours <= 24 && floor(var.cognito_m2m_default_access_token_hours) == var.cognito_m2m_default_access_token_hours
    error_message = "cognito_m2m_default_access_token_hours must be a whole number between 1 and 24."
  }
}

variable "cognito_m2m_organizations" {
  description = "Per-organization confidential M2M clients keyed by an opaque lowercase Terraform-safe key. allowed_scopes are custom-scope suffixes; 24-hour enabled clients require recorded risk acceptance metadata."
  type = map(object({
    display_name       = string
    enabled            = bool
    allowed_scopes     = set(string)
    access_token_hours = optional(number, null)
    risk_acceptance = optional(object({
      accepted_by = string
      accepted_at = string
      ticket      = string
    }), null)
  }))
  default = {}

  validation {
    condition = alltrue([
      for key, organization in var.cognito_m2m_organizations :
      can(regex("^[a-z0-9][a-z0-9-]{0,62}$", key)) &&
      length(trimspace(organization.display_name)) > 0 &&
      (!organization.enabled || length(organization.allowed_scopes) > 0) &&
      alltrue([
        for scope in organization.allowed_scopes :
        contains(["external.read", "external.index", "external.status"], scope)
      ]) &&
      (
        organization.access_token_hours == null ||
        (organization.access_token_hours >= 1 && organization.access_token_hours <= 24 && floor(organization.access_token_hours) == organization.access_token_hours)
      ) &&
      (
        !organization.enabled ||
        coalesce(organization.access_token_hours, var.cognito_m2m_default_access_token_hours) < 24 ||
        (
          organization.risk_acceptance != null &&
          length(trimspace(organization.risk_acceptance.accepted_by)) > 0 &&
          length(trimspace(organization.risk_acceptance.ticket)) > 0 &&
          can(formatdate("YYYY-MM-DD'T'hh:mm:ssZ", organization.risk_acceptance.accepted_at))
        )
      )
    ])
    error_message = "Each Cognito M2M organization needs a Terraform-safe key, nonblank display name, least-privilege valid scope subset, valid 1-24 hour TTL, and accepted_by/accepted_at/ticket metadata for an enabled 24-hour client."
  }

  validation {
    condition = !var.cognito_auth_enabled || (
      1 + length([for organization in values(var.cognito_m2m_organizations) : organization if organization.enabled]) <= 50
    )
    error_message = "Cognito JWT authorizer audiences are limited to one human client plus at most 49 enabled M2M organizations."
  }
}

variable "cognito_emergency_deny_client_ids" {
  description = "Restricted, non-secret emergency denylist of Cognito client IDs that Lambda rejects after deployment. Do not commit real client IDs."
  type        = set(string)
  default     = []
  sensitive   = true

  validation {
    condition     = alltrue([for client_id in var.cognito_emergency_deny_client_ids : length(trimspace(client_id)) > 0 && length(client_id) <= 256])
    error_message = "cognito_emergency_deny_client_ids entries must be nonblank values of at most 256 characters."
  }
}

variable "cognito_monthly_budget_usd" {
  description = "Optional positive monthly Cognito cost budget in USD. A budget resource is created only when Cognito is enabled and subscribers are configured."
  type        = number
  default     = null

  validation {
    condition     = var.cognito_monthly_budget_usd == null || var.cognito_monthly_budget_usd > 0
    error_message = "cognito_monthly_budget_usd must be null or greater than zero."
  }

  validation {
    condition     = var.cognito_monthly_budget_usd == null || nonsensitive(length(var.cognito_budget_subscriber_emails)) > 0
    error_message = "cognito_monthly_budget_usd requires at least one cognito_budget_subscriber_email."
  }
}

variable "cognito_budget_subscriber_emails" {
  description = "Sensitive budget-notification email addresses. This is personal contact data, not a Cognito credential."
  type        = set(string)
  default     = []
  sensitive   = true

  validation {
    condition = alltrue([
      for email in var.cognito_budget_subscriber_emails :
      can(regex("^[^@\\s]+@[^@\\s]+\\.[^@\\s]+$", email))
    ])
    error_message = "cognito_budget_subscriber_emails must contain valid nonblank email addresses."
  }
}

variable "context_ducklake_data_path" {
  description = "DuckLake data path used by worker translate jobs. Must end in /gold/data so the frozen snapshot pointer lands at s3://<bucket>/gold/catalog-snapshot/current.json. Defaults to s3://<bucket_name>/gold/data/."
  type        = string
  default     = null
}

variable "aurora_cluster_identifier" {
  description = "Aurora Serverless v2 cluster identifier for the live DuckLake ingest catalog"
  type        = string
  default     = "spur-context-catalog"
}

variable "aurora_database_name" {
  description = "Postgres database name for the live DuckLake ingest catalog"
  type        = string
  default     = "spur_context"
}

variable "aurora_master_username" {
  description = "Aurora master username. The password is generated and stored by RDS in Secrets Manager."
  type        = string
  default     = "spur_context"
}

variable "aurora_engine_version" {
  description = "Aurora PostgreSQL engine version. Null lets RDS choose the regional default."
  type        = string
  default     = null
}

variable "aurora_subnets" {
  description = "Private subnet IDs for Aurora. Defaults to worker_subnets when null."
  type        = list(string)
  default     = null
}

variable "aurora_max_acu" {
  description = "Maximum Aurora Serverless v2 capacity in ACUs"
  type        = number
  default     = 4
}

variable "aurora_seconds_until_auto_pause" {
  description = "Seconds of inactivity before Aurora Serverless v2 auto-pauses at 0 ACU"
  type        = number
  default     = 300
}

variable "aurora_backup_retention_days" {
  description = "Aurora backup retention period in days"
  type        = number
  default     = 7
}

variable "aurora_deletion_protection" {
  description = "Enable deletion protection on the Aurora catalog cluster"
  type        = bool
  default     = true
}

variable "index_jobs_table_name" {
  description = "DynamoDB table name for context-service index job records and dedupe pointers"
  type        = string
  default     = "spur-context-index-jobs"
}

variable "catalog_leases_table_name" {
  description = "DynamoDB table name for context-service catalog write leases"
  type        = string
  default     = "spur-context-catalog-leases"
}

variable "code_lambda_memory_mb" {
  description = "DuckDB-free Code Lambda memory allocation"
  type        = number
  default     = 256
}

variable "lambda_memory_mb" {
  description = "Knowledge and legacy compatibility Lambda memory allocation"
  type        = number
  default     = 1024
}

variable "lambda_timeout_sec" {
  description = "Lambda timeout in seconds"
  type        = number
  default     = 30
}

variable "lambda_max_concurrency" {
  description = "Maximum in-process request concurrency per serving Lambda environment (1 = sequential)"
  type        = number
  default     = 4
}

variable "code_lambda_ephemeral_storage_mb" {
  description = "Code Lambda /tmp capacity in MiB. Fixed to AWS Lambda's 512 MiB platform minimum; cache bytes are derived exactly with no separate reserve."
  type        = number
  default     = 512

  validation {
    condition     = var.code_lambda_ephemeral_storage_mb == 512
    error_message = "code_lambda_ephemeral_storage_mb must remain at the AWS Lambda platform minimum of 512 MiB."
  }
}

variable "concurrent_warm_instances" {
  description = "Existing billable provisioned-concurrency budget, attached only to Knowledge (0 disables the warm pool)"
  type        = number
  default     = 0

  validation {
    condition     = var.concurrent_warm_instances >= 0 && var.concurrent_warm_instances <= 1 && floor(var.concurrent_warm_instances) == var.concurrent_warm_instances
    error_message = "concurrent_warm_instances must be either 0 or 1."
  }
}

# ─── Public API Abuse Controls ────────────────────────────────────────────────

variable "api_throttle_rate_limit" {
  description = "API Gateway account-level route throttle rate in requests per second"
  type        = number
  default     = 20
}

variable "api_throttle_burst_limit" {
  description = "API Gateway account-level route throttle burst"
  type        = number
  default     = 40
}

variable "index_rate_limit_per_minute" {
  description = "Per authenticated caller external_index fixed-window rate limit"
  type        = number
  default     = 10
}

variable "index_max_concurrent_jobs_per_caller" {
  description = "Legacy per-caller running cap used when index_max_running_jobs_per_owner is null."
  type        = number
  default     = 2
}

# ─── Bounded Backlog Queueing ────────────────────────────────────────────────
# Config surface for the DynamoDB backlog/backpressure design. Defaults keep
# durable queue admission disabled until an operator sets a finite queue cap.
# See
# docs/superpowers/specs/2026-07-10-context-service-index-queue-backpressure-design.md

variable "index_queue_gsi_name" {
  description = "Name of the sparse DynamoDB GSI keyed by (queue_shard, queue_sort_key) used by the drainer to scan queued jobs in FIFO order."
  type        = string
  default     = "queue-gsi"
}

variable "index_queue_shard_count" {
  description = "Number of shards for the sparse queue GSI partition key. The drainer rotates the starting shard between invocations. Default 16 per the design spec."
  type        = number
  default     = 16

  validation {
    condition     = var.index_queue_shard_count > 0 && var.index_queue_shard_count <= 1024 && floor(var.index_queue_shard_count) == var.index_queue_shard_count
    error_message = "index_queue_shard_count must be a whole number between 1 and 1024."
  }
}

variable "index_max_running_jobs_per_owner" {
  description = "Maximum concurrent running/dispatching index jobs per backlog owner. Null inherits index_max_concurrent_jobs_per_caller for backward-compatible deployments."
  type        = number
  default     = null

  validation {
    condition     = var.index_max_running_jobs_per_owner == null || (var.index_max_running_jobs_per_owner >= 0 && floor(var.index_max_running_jobs_per_owner) == var.index_max_running_jobs_per_owner)
    error_message = "index_max_running_jobs_per_owner must be null or a non-negative integer."
  }
}

variable "index_max_queued_jobs_per_owner" {
  description = "Maximum accepted queued backlog per backlog owner. 0 (default) rejects cold admissions until queueing is explicitly enabled."
  type        = number
  default     = 0

  validation {
    condition     = var.index_max_queued_jobs_per_owner >= 0 && floor(var.index_max_queued_jobs_per_owner) == var.index_max_queued_jobs_per_owner
    error_message = "index_max_queued_jobs_per_owner must be a non-negative integer."
  }
}

variable "index_max_running_jobs_global" {
  description = "Global concurrent running/dispatching job cap enforced via a RUNNING# token pool. 0 (default) disables the hard global running cap; small deployments rely on per-owner caps plus API Gateway throttles."
  type        = number
  default     = 0

  validation {
    condition     = var.index_max_running_jobs_global >= 0 && var.index_max_running_jobs_global <= 32 && floor(var.index_max_running_jobs_global) == var.index_max_running_jobs_global
    error_message = "index_max_running_jobs_global must be a whole number between 0 and 32."
  }
}

variable "index_max_queued_jobs_global" {
  description = "Global accepted queued backlog, enforced via sharded GLOBAL#QUEUE# counters (conservative/approximate under contention). 0 (default) disables the global queued cap."
  type        = number
  default     = 0

  validation {
    condition     = var.index_max_queued_jobs_global >= 0 && floor(var.index_max_queued_jobs_global) == var.index_max_queued_jobs_global
    error_message = "index_max_queued_jobs_global must be a non-negative integer."
  }
}

variable "index_drainer_batch_limit" {
  description = "Maximum jobs dispatched by one scheduled or admission-kick drainer invocation."
  type        = number
  default     = 8

  validation {
    condition     = var.index_drainer_batch_limit >= 1 && floor(var.index_drainer_batch_limit) == var.index_drainer_batch_limit
    error_message = "index_drainer_batch_limit must be a positive integer."
  }
}

variable "index_drainer_scan_limit_per_shard" {
  description = "Maximum queued candidates queried from each queue GSI shard per drainer invocation."
  type        = number
  default     = 32

  validation {
    condition     = var.index_drainer_scan_limit_per_shard >= 1 && floor(var.index_drainer_scan_limit_per_shard) == var.index_drainer_scan_limit_per_shard
    error_message = "index_drainer_scan_limit_per_shard must be a positive integer."
  }
}

variable "index_drainer_schedule_rate_minutes" {
  description = "EventBridge correctness-drainer cadence in whole minutes. The same value drives runtime shard-start rotation so every shard is visited even when cadence and shard count share factors."
  type        = number
  default     = 1

  validation {
    condition     = var.index_drainer_schedule_rate_minutes >= 1 && floor(var.index_drainer_schedule_rate_minutes) == var.index_drainer_schedule_rate_minutes
    error_message = "index_drainer_schedule_rate_minutes must be a positive whole number of minutes."
  }
}

variable "index_dispatch_max_attempts" {
  description = "Maximum transient-dispatch retry attempts before a queued job is marked failed with error_code=dispatch_exhausted."
  type        = number
  default     = 3

  validation {
    condition     = var.index_dispatch_max_attempts > 0
    error_message = "index_dispatch_max_attempts must be greater than zero."
  }
}

variable "index_dispatch_backoff_base_seconds" {
  description = "Base seconds for exponential backoff when re-queuing a job after a transient dispatch failure. The actual backoff is base * 2^(attempt-1), capped at a sane maximum."
  type        = number
  default     = 5

  validation {
    condition     = var.index_dispatch_backoff_base_seconds > 0
    error_message = "index_dispatch_backoff_base_seconds must be greater than zero."
  }
}

variable "context_max_tarball_bytes" {
  description = "Maximum downloaded tarball bytes for external_index"
  type        = number
  default     = 524288000
}

variable "context_max_git_bytes" {
  description = "Maximum fetched git source tree bytes for external_index"
  type        = number
  default     = 2147483648
}

variable "context_max_build_seconds" {
  description = "Maximum spur graph build runtime for an indexing worker"
  type        = number
  default     = 1800
}

variable "allowed_source_domains" {
  description = "Optional source_url domain allow-list for external_index; empty allows public non-private domains"
  type        = list(string)
  default     = []
}

# ─── On-Demand Indexing ──────────────────────────────────────────────────────

variable "vpc_id" {
  description = "VPC ID for the ECS worker tasks (needs S3/RDS/SFN egress). Empty (default) discovers the account default VPC via data.aws_vpc.selected."
  type        = string
  default     = ""
}

variable "worker_subnets" {
  description = "Private subnets for Lambda and ECS worker tasks. Empty (default) discovers all subnets of the selected VPC. By default this module creates NAT-free VPC endpoints in these subnets."
  type        = list(string)
  default     = []
}

variable "interface_vpc_endpoint_subnet_ids" {
  description = "Subnet IDs for interface VPC endpoint ENIs. Empty (default) reuses worker_subnets/all discovered worker subnets; set one subnet for low-cost dev stacks."
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for subnet_id in var.interface_vpc_endpoint_subnet_ids : length(trimspace(subnet_id)) > 0])
    error_message = "interface_vpc_endpoint_subnet_ids entries must be non-empty subnet IDs."
  }
}

variable "interface_vpc_endpoint_service_keys" {
  description = "Interface VPC endpoint service keys to create. Use [\"states\", \"secretsmanager\"] for Lambda-worker-only low-cost stacks; add ecr_api/ecr_dkr/logs/sts when private ECS fallback tasks need NAT-free ECR pull, CloudWatch Logs, or STS access."
  type        = set(string)
  default     = ["states", "secretsmanager", "ecr_api", "ecr_dkr", "logs", "sts"]

  validation {
    condition = alltrue([
      for service_key in var.interface_vpc_endpoint_service_keys :
      contains(["states", "secretsmanager", "ecr_api", "ecr_dkr", "logs", "sts"], service_key)
    ])
    error_message = "interface_vpc_endpoint_service_keys entries must be one of states, secretsmanager, ecr_api, ecr_dkr, logs, sts."
  }
}

variable "worker_route_table_ids" {
  description = "Route table IDs associated with worker_subnets for S3 and DynamoDB gateway endpoints. Required when create_vpc_endpoints is true."
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for route_table_id in var.worker_route_table_ids : length(trimspace(route_table_id)) > 0])
    error_message = "worker_route_table_ids entries must be non-empty route table IDs."
  }
}

variable "vpc_endpoint_extra_client_sg_ids" {
  description = "Extra security group IDs (beyond the worker SG) allowed inbound 443 on the interface VPC endpoints. Needed for other clients sharing this VPC that rely on the endpoints' VPC-wide private DNS, e.g. the spur cloud-build VM in the default VPC."
  type        = list(string)
  default     = []
}

variable "create_vpc_endpoints" {
  description = "Create NAT-free VPC endpoints for worker access to S3, DynamoDB, Step Functions, Secrets Manager, ECR, CloudWatch Logs, and STS. Disable only when worker_subnets already have equivalent NAT or endpoints."
  type        = bool
  default     = true
}

variable "vpc_endpoint_region" {
  description = "Optional region override for AWS VPC endpoint service names. Defaults to aws_region."
  type        = string
  default     = null

  validation {
    condition     = var.vpc_endpoint_region == null ? true : length(trimspace(var.vpc_endpoint_region)) > 0
    error_message = "vpc_endpoint_region must be null or a non-empty region name."
  }
}

variable "worker_ecr_image" {
  description = "ECR image URI for the spur-context-worker container (e.g. <acct>.dkr.ecr.<region>.amazonaws.com/spur-context-worker:latest)"
  type        = string
}

variable "worker_lambda_image" {
  description = "ECR image URI for the Lambda-compatible spur-context-worker image"
  type        = string
}

variable "source_fetcher_lambda_image" {
  description = "ECR image URI for the non-VPC source fetcher Lambda image"
  type        = string
}

variable "manage_ecr_lifecycle_policies" {
  description = "Manage ECR lifecycle policies for context-service worker image repositories."
  type        = bool
  default     = true
}

variable "ecr_lifecycle_repository_names" {
  description = "Existing ECR repository names that should receive the context-service cleanup lifecycle policy."
  type        = set(string)
  default = [
    "spur-context-worker",
    "spur-context-worker-lambda",
    "spur-context-source-fetcher",
  ]

  validation {
    condition     = alltrue([for repository_name in var.ecr_lifecycle_repository_names : length(trimspace(repository_name)) > 0])
    error_message = "ecr_lifecycle_repository_names entries must be non-empty ECR repository names."
  }
}

variable "ecr_lifecycle_keep_tagged_images" {
  description = "Number of tagged images to retain in each context-service ECR repository."
  type        = number
  default     = 10

  validation {
    condition     = var.ecr_lifecycle_keep_tagged_images > 0
    error_message = "ecr_lifecycle_keep_tagged_images must be greater than zero."
  }
}

variable "ecr_lifecycle_untagged_image_days" {
  description = "Expire untagged ECR images older than this many days."
  type        = number
  default     = 7

  validation {
    condition     = var.ecr_lifecycle_untagged_image_days > 0
    error_message = "ecr_lifecycle_untagged_image_days must be greater than zero."
  }
}

variable "worker_lambda_memory_mb" {
  description = "Lambda worker memory allocation. This account/region currently accepts up to 3008 MB; raise after a Lambda memory quota increase."
  type        = number
  default     = 3008
}

variable "worker_lambda_timeout_sec" {
  description = "Lambda worker timeout in seconds. Lambda max is 900 seconds."
  type        = number
  default     = 900
}

variable "worker_lambda_ephemeral_storage_mb" {
  description = "Lambda worker /tmp storage in MB. Lambda max is 10240 MB."
  type        = number
  default     = 10240
}

variable "worker_lambda_provisioned_concurrency" {
  description = "Provisioned concurrency for the Lambda worker live alias (0 = disabled)"
  type        = number
  default     = 0
}

variable "source_fetcher_lambda_timeout_sec" {
  description = "Source fetcher Lambda timeout in seconds. Lambda max is 900 seconds."
  type        = number
  default     = 900
}

variable "source_fetcher_lambda_memory_mb" {
  description = "Source fetcher Lambda memory allocation"
  type        = number
  default     = 1024
}

variable "source_fetcher_lambda_ephemeral_storage_mb" {
  description = "Source fetcher Lambda /tmp storage in MB. Lambda max is 10240 MB."
  type        = number
  default     = 10240
}

variable "source_fetch_presign_seconds" {
  description = "Validity period in seconds for presigned fetch artifact URLs returned to workers"
  type        = number
  default     = 21600
}

variable "fetch_artifact_retention_days" {
  description = "Number of days to retain staged fetch artifacts under s3://<bucket>/fetch/"
  type        = number
  default     = 7
}

locals {
  # Must end in `/gold/data` so catalog.rs `snapshot_base_uri` strips that
  # suffix to derive the bucket root and writes the frozen snapshot pointer at
  # `s3://<bucket>/gold/catalog-snapshot/current.json` — matching the serving
  # `catalog_s3_uri` default. A bare `.../data/` path offsets the entire gold
  # layer one level deep (`.../data/gold/...`) and serving never finds it.
  context_ducklake_data_path     = coalesce(var.context_ducklake_data_path, "s3://${var.bucket_name}/gold/data/")
  worker_checkpoint_uri_template = "s3://${var.bucket_name}/jobs/{}/checkpoint.json"
  aurora_subnet_ids              = var.aurora_subnets != null ? var.aurora_subnets : local.net_subnet_ids
  # A long-idle Aurora resume took 74s in production. Override the client's 30s
  # default for both Lambda and ECS workers, including Step Functions overrides.
  aurora_catalog_dsn              = "postgres:host=${aws_rds_cluster.catalog.endpoint} port=${aws_rds_cluster.catalog.port} dbname=${var.aurora_database_name} user=${var.aurora_master_username} sslmode=require connect_timeout=120"
  aurora_master_secret_arn        = aws_rds_cluster.catalog.master_user_secret[0].secret_arn
  aurora_master_password_valuearn = "${local.aurora_master_secret_arn}:password::"

  cognito_external_scope_descriptions = {
    "external.read"   = "Read external context-service catalog and code data"
    "external.index"  = "Submit external context-service indexing jobs"
    "external.status" = "Read external context-service indexing job status"
  }
  cognito_scope_descriptions = merge(
    local.cognito_external_scope_descriptions,
    var.api_key_auth_enabled ? {
      "keys.manage" = "Create, list, and revoke personal SPUR context-service API keys"
    } : {},
  )
  cognito_custom_scopes = [
    for suffix in keys(local.cognito_external_scope_descriptions) :
    "${var.cognito_resource_server_identifier}/${suffix}"
  ]
  cognito_human_allowed_oauth_scopes = concat(
    tolist(var.cognito_human_oidc_scopes),
    [
      for suffix in var.cognito_human_custom_scopes :
      "${var.cognito_resource_server_identifier}/${suffix}"
    ],
    var.api_key_auth_enabled ? [
      "${var.cognito_resource_server_identifier}/keys.manage"
    ] : [],
  )
  api_key_management_scope = "${var.cognito_resource_server_identifier}/keys.manage"
  api_key_authorizer_dynamodb_actions = [
    "dynamodb:GetItem",
  ]
  api_key_management_dynamodb_actions = [
    "dynamodb:GetItem",
    "dynamodb:PutItem",
    "dynamodb:UpdateItem",
    "dynamodb:TransactWriteItems",
  ]
  api_key_management_query_actions = [
    "dynamodb:Query",
  ]
  api_key_cleanup_dynamodb_actions = [
    "dynamodb:GetItem",
    "dynamodb:UpdateItem",
    "dynamodb:TransactWriteItems",
  ]
  api_key_cleanup_query_actions = [
    "dynamodb:Query",
  ]
  cognito_enabled_m2m_organizations = var.cognito_auth_enabled ? {
    for key, organization in var.cognito_m2m_organizations : key => organization
    if organization.enabled
  } : {}
  cognito_m2m_access_token_hours = {
    for key, organization in var.cognito_m2m_organizations :
    key => coalesce(organization.access_token_hours, var.cognito_m2m_default_access_token_hours)
  }
  context_service_domain_name = "context.getspur.dev"
  cognito_custom_domain_name  = "auth.context.getspur.dev"
  context_service_base_url = var.custom_domains_enabled ? (
    "https://${local.context_service_domain_name}"
  ) : aws_apigatewayv2_api.http.api_endpoint
  cognito_issuer = var.cognito_auth_enabled ? "https://cognito-idp.${var.aws_region}.amazonaws.com/${aws_cognito_user_pool.context_service[0].id}" : ""
  cognito_domain_url = var.cognito_auth_enabled ? (
    var.custom_domains_enabled ?
    "https://${local.cognito_custom_domain_name}" :
    "https://${aws_cognito_user_pool_domain.context_service[0].domain}.auth.${var.aws_region}.amazoncognito.com"
  ) : ""
}
