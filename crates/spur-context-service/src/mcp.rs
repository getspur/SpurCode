//! MCP tool definitions and handlers for the external code context service.

use std::env;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_sfn::types::ExecutionStatus as AwsExecutionStatus;
use duckdb::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::abuse::{self, SourceKind, ValidateOptions};
use crate::catalog::{readable_table, CatalogResolver, ResolvedRevision};
use crate::code_backend::{
    CatalogRequest, CodeBackend, CodeBackendError, CodeEdgesRequest, CodeReadRequest,
    CodeSearchRequest,
};
use crate::jobs::{
    BacklogOwner, CreateJobRequest, EnqueueOutcome, JobRecord, JobStatus, JobStore, JobsError,
    QueueConfig,
};
use crate::knowledge::{self, KnowledgeContextOptions, KnowledgeScope};
use crate::query::{self, SearchMode, SearchOptions};

const DEFAULT_SOURCE: &str = "registry:crates-io";
const DEFAULT_INDEX_SOURCE: &str = "git:custom";
const DEFAULT_REF: &str = "latest";
const KNOWLEDGE_QUERY_VECTOR_DIMENSIONS: usize = 768;
const RATE_LIMIT_RETRY_AFTER_SECONDS: u64 = 60;
const DEFAULT_TARBALL_SIZE_CAP_BYTES: u64 = 500_u64 * 1024 * 1024;
const DEFAULT_GIT_SIZE_CAP_BYTES: u64 = 2_u64 * 1024 * 1024 * 1024;
const DEFAULT_MAX_BUILD_SECONDS: u64 = 30 * 60;
const DEFAULT_MAX_CONCURRENT_JOBS_PER_CALLER: u32 = 2;
const DEFAULT_CALLS_PER_MINUTE: u32 = 10;
const ENQUEUE_CONFLICT_MAX_RETRIES: u32 = 10;
const ENQUEUE_CONFLICT_BACKOFF_BASE_MS: u64 = 25;
const ENQUEUE_CONFLICT_BACKOFF_MAX_MS: u64 = 500;
const DESCRIBE_EXECUTION_TIMEOUT: Duration = Duration::from_secs(2);
const STALE_JOB_REPAIR_AFTER: Duration = Duration::from_secs(60);

/// Metadata for a single context-service MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexExecutionRequest {
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct IndexResourceLimits {
    pub max_source_bytes: u64,
    pub max_build_seconds: u64,
}

pub trait IndexExecutionStarter: Send + Sync {
    fn start_execution<'a>(
        &'a self,
        request: IndexExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<String, McpHandlerError>> + Send + 'a>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutcomeStatus {
    Succeeded,
    Failed,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub status: ExecutionOutcomeStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
}

pub trait ExecutionStatusChecker: Send + Sync {
    fn describe_execution(&self, arn: &str) -> Result<Option<ExecutionOutcome>, McpHandlerError>;
}

#[derive(Debug, Clone)]
pub struct SfnExecutionStatusChecker {
    client: aws_sdk_sfn::Client,
    timeout: Duration,
}

impl SfnExecutionStatusChecker {
    pub fn new(client: aws_sdk_sfn::Client) -> Self {
        Self {
            client,
            timeout: DESCRIBE_EXECUTION_TIMEOUT,
        }
    }

    pub fn with_timeout(client: aws_sdk_sfn::Client, timeout: Duration) -> Self {
        Self { client, timeout }
    }
}

impl ExecutionStatusChecker for SfnExecutionStatusChecker {
    fn describe_execution(&self, arn: &str) -> Result<Option<ExecutionOutcome>, McpHandlerError> {
        let client = self.client.clone();
        let arn = arn.to_owned();
        let timeout = self.timeout;
        let output = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().map_err(|error| {
                McpHandlerError::Internal(format!(
                    "DescribeExecution runtime creation failed: {error}"
                ))
            })?;
            runtime.block_on(async move {
                tokio::time::timeout(
                    timeout,
                    client.describe_execution().execution_arn(arn).send(),
                )
                .await
                .map_err(|error| {
                    McpHandlerError::Internal(format!("DescribeExecution timed out: {error}"))
                })?
                .map_err(|error| McpHandlerError::Internal(format!("DescribeExecution: {error}")))
            })
        })
        .join()
        .map_err(|_panic| {
            McpHandlerError::Internal("DescribeExecution thread panicked".to_owned())
        })??;

        sfn_execution_outcome(output).map(Some)
    }
}

#[derive(Debug, Error)]
pub enum McpHandlerError {
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl McpHandlerError {
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::NotFound(_) => -32004,
            Self::Internal(_) => -32603,
        }
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
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

#[expect(
    clippy::future_not_send,
    clippy::unused_async,
    reason = "public MCP entry point is required to be async while the DuckDB-backed implementation is synchronous"
)]
pub async fn handle_tool(
    name: &str,
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    handle_tool_sync(name, args, db, catalog)
}

pub fn handle_tool_sync(
    name: &str,
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    match name {
        "external_catalog" => handle_catalog(args, db, catalog),
        "external_code_search" => handle_code_search(args, db, catalog),
        "external_code_read" => handle_code_read(args, db, catalog),
        "external_code_callers" => handle_code_callers(args, db, catalog),
        "external_code_callees" => handle_code_callees(args, db, catalog),
        "external_knowledge_context" => handle_knowledge_context(args, db, catalog),
        "external_index" => handle_index_requires_lambda(args),
        "external_index_status" => handle_index_status_requires_lambda(args),
        other => Err(McpHandlerError::InvalidParams(format!(
            "unknown context-service MCP tool: {other}"
        ))),
    }
}

pub fn handle_tool_without_catalog(name: &str, args: &Value) -> Result<Value, McpHandlerError> {
    match name {
        "external_catalog" => handle_catalog_without_catalog(args),
        "external_code_search" => handle_code_search_without_catalog(args),
        "external_code_read" => handle_code_read_without_catalog(args),
        "external_code_callers" => handle_code_callers_without_catalog(args),
        "external_code_callees" => handle_code_callees_without_catalog(args),
        "external_knowledge_context" => handle_knowledge_context_without_catalog(args),
        "external_index" => handle_index_requires_lambda(args),
        "external_index_status" => handle_index_status_requires_lambda(args),
        other => Err(McpHandlerError::InvalidParams(format!(
            "unknown context-service MCP tool: {other}"
        ))),
    }
}

/// Routes the immutable Parquet-backed subset of the MCP surface.
pub async fn handle_tool_with_code_backend(
    name: &str,
    args: &Value,
    backend: &CodeBackend,
) -> Result<Value, McpHandlerError> {
    match name {
        "external_catalog" => {
            let args: CatalogArgs = parse_args(args)?;
            args.validate()?;
            if args.package.is_none()
                && (args.revision.is_some() || args.ref_name.is_some() || args.path.is_some())
            {
                return Err(McpHandlerError::InvalidParams(
                    "field 'package' is required when using revision, ref, or path".to_owned(),
                ));
            }
            if args.package.is_some() && args.revision_ref().is_none() && args.path.is_some() {
                return Err(McpHandlerError::InvalidParams(
                    "field 'path' requires revision or ref".to_owned(),
                ));
            }
            let request = CatalogRequest {
                source: args.source().to_owned(),
                package: args.package.clone(),
                revision_or_ref: args.revision_ref().map(str::to_owned),
                path: args.path.clone(),
                name_filter: args.name_filter.clone(),
                limit: args.limit(),
                cursor: args.cursor.clone(),
            };
            backend.catalog(request).await.map_err(code_backend_error)
        }
        "external_code_search" => {
            let args: CodeSearchArgs = parse_args(args)?;
            args.validate()?;
            let request = CodeSearchRequest {
                source: args.source().to_owned(),
                package: args.package.clone(),
                revision_or_ref: args.revision_ref().map(str::to_owned),
                query: args.query.clone(),
                symbol_kind: args.symbol_kind.clone(),
                limit: args.limit.unwrap_or(20).clamp(1, 200),
            };
            backend.search(request).await.map_err(code_backend_error)
        }
        "external_code_read" => {
            let args: CodeReadArgs = parse_args(args)?;
            backend
                .read(CodeReadRequest {
                    source: args.source().to_owned(),
                    selector: args.selector,
                    context_lines: args.context_lines.unwrap_or(0),
                })
                .await
                .map_err(|error| code_backend_tool_error("external_code_read failed", error))
        }
        "external_code_callers" => {
            let args: CodeCallersArgs = parse_args(args)?;
            backend
                .callers(CodeEdgesRequest {
                    source: args.source().to_owned(),
                    selector: args.selector,
                    include_unresolved: args.include_unresolved.unwrap_or(false),
                })
                .await
                .map_err(|error| code_backend_tool_error("external_code_callers failed", error))
        }
        "external_code_callees" => {
            let args: CodeCalleesArgs = parse_args(args)?;
            backend
                .callees(CodeEdgesRequest {
                    source: args.source().to_owned(),
                    selector: args.selector,
                    include_unresolved: args.include_unresolved.unwrap_or(false),
                })
                .await
                .map_err(|error| code_backend_tool_error("external_code_callees failed", error))
        }
        "external_index" => handle_index_with_code_backend(args, backend).await,
        "external_index_status" => {
            let args: ExternalIndexStatusArgs = parse_args(args)?;
            args.validate()?;
            backend
                .index_status(&args.job_id)
                .await
                .map_err(code_backend_error)
        }
        _ => handle_tool_without_catalog(name, args),
    }
}

async fn handle_index_with_code_backend(
    args: &Value,
    backend: &CodeBackend,
) -> Result<Value, McpHandlerError> {
    let parsed_args: ExternalIndexArgs = parse_args(args)?;
    parsed_args.validate()?;
    let parsed_url = match abuse::validate(parsed_args.source_url.trim(), &index_validate_options())
    {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(json!({
                "status": "rejected",
                "reason": format!("source_url: {error}")
            }));
        }
    };
    let identity = match ExternalIndexIdentity::canonicalize(&parsed_args, &parsed_url) {
        Ok(identity) => identity,
        Err(reason) => return Ok(json!({ "status": "rejected", "reason": reason })),
    };
    if parsed_args.force.unwrap_or(false) {
        return handle_index_requires_lambda(args);
    }
    if let Some(response) = backend
        .warm_index(&identity.source, &identity.package, &identity.revision)
        .await
        .map_err(code_backend_error)?
    {
        return Ok(response);
    }
    if let Some(legacy_source) = identity.legacy_warm_source.as_deref() {
        if let Some(response) = backend
            .warm_index(legacy_source, &identity.package, &identity.revision)
            .await
            .map_err(code_backend_error)?
        {
            return Ok(response);
        }
    }
    handle_index_requires_lambda(args)
}

fn code_backend_error(error: CodeBackendError) -> McpHandlerError {
    match error {
        CodeBackendError::PackageUnavailable => {
            McpHandlerError::NotFound("package revision not found".to_owned())
        }
        CodeBackendError::InvalidCursor => {
            McpHandlerError::InvalidParams("invalid catalog cursor".to_owned())
        }
        error => McpHandlerError::Internal(format!(
            "code backend temporarily unavailable: {} (retryable={})",
            error.code(),
            error.is_retryable()
        )),
    }
}

fn code_backend_tool_error(context: &'static str, error: CodeBackendError) -> McpHandlerError {
    match error {
        CodeBackendError::InvalidSelector(message) => McpHandlerError::InvalidParams(message),
        CodeBackendError::AmbiguousSelector { .. } => {
            McpHandlerError::InvalidParams(format!("{context}: {error}"))
        }
        CodeBackendError::SymbolNotFound(selector) => {
            McpHandlerError::NotFound(format!("symbol not found: {selector}"))
        }
        error => code_backend_error(error),
    }
}

fn handle_catalog(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: CatalogArgs = parse_args(args)?;
    args.validate()?;
    let source = args.source();
    let limit = args.limit();
    let cursor = args.cursor.as_deref();

    let Some(package) = args.package.as_deref() else {
        if args.revision.is_some() || args.ref_name.is_some() || args.path.is_some() {
            return Err(McpHandlerError::InvalidParams(
                "field 'package' is required when using revision, ref, or path".to_owned(),
            ));
        }
        return json_value(
            query::list_packages(db, source, args.name_filter.as_deref(), limit, cursor)
                .map_err(internal_error("external_catalog packages failed"))?,
        );
    };

    if args.revision_ref().is_none() {
        if args.path.is_some() {
            return Err(McpHandlerError::InvalidParams(
                "field 'path' requires revision or ref".to_owned(),
            ));
        }
        return json_value(
            query::list_revisions_and_refs(db, source, package, limit, cursor)
                .map_err(internal_error("external_catalog revisions failed"))?,
        );
    }

    let resolved = resolve_revision(catalog, source, package, args.revision_ref())?;
    if let Some(path) = args.path.as_deref() {
        if query::catalog_file_exists(
            db,
            &resolved.source,
            &resolved.package,
            &resolved.revision,
            path,
        )
        .map_err(internal_error("external_catalog file lookup failed"))?
        {
            return json_value(
                query::list_file_symbols(
                    db,
                    &resolved.source,
                    &resolved.package,
                    &resolved.revision,
                    path,
                    args.name_filter.as_deref(),
                    limit,
                    cursor,
                )
                .map_err(internal_error("external_catalog symbols failed"))?,
            );
        }
    }

    json_value(
        query::list_tree_entries(
            db,
            &resolved.source,
            &resolved.package,
            &resolved.revision,
            args.path.as_deref(),
            limit,
            cursor,
        )
        .map_err(internal_error("external_catalog tree failed"))?,
    )
}

fn handle_catalog_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let args: CatalogArgs = parse_args(args)?;
    args.validate()?;
    json_value(query::CatalogPage::<query::PackageCatalogRow> {
        level: query::CatalogLevel::Packages,
        rows: Vec::new(),
        total_matches: 0,
        truncated: false,
        next_cursor: None,
        catalog_generation: None,
    })
}

fn handle_code_search(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: CodeSearchArgs = parse_args(args)?;
    args.validate()?;
    let source = args.source();
    let resolved = resolve_revision(catalog, source, &args.package, args.revision_ref())?;
    let result = query::search_symbols(
        db,
        &SearchOptions {
            source: resolved.source,
            package: resolved.package,
            revision: resolved.revision,
            query: args.query,
            mode: SearchMode::Substring,
            symbol_kind: args.symbol_kind,
            file_glob: None,
            limit: args.limit.unwrap_or(20),
        },
    )
    .map_err(internal_error("external_code_search failed"))?;
    json_value(result)
}

fn handle_code_search_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let args: CodeSearchArgs = parse_args(args)?;
    args.validate()?;
    json_value(query::CodeSearchResult {
        candidates: Vec::new(),
        total_matches: 0,
        truncated: false,
    })
}

fn handle_code_read(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: CodeReadArgs = parse_args(args)?;
    let selector = normalize_selector(&args.selector, args.source(), catalog)?;
    let source = query::read_symbol(db, &selector, args.context_lines.unwrap_or(0))
        .map_err(selector_query_error("external_code_read failed"))?
        .ok_or_else(|| McpHandlerError::NotFound(format!("symbol not found: {}", args.selector)))?;
    json_value(source)
}

fn handle_code_read_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let _args: CodeReadArgs = parse_args(args)?;
    Ok(Value::Null)
}

fn handle_code_callers(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: CodeCallersArgs = parse_args(args)?;
    let selector = normalize_selector(&args.selector, args.source(), catalog)?;
    let result = query::find_callers(db, &selector, args.include_unresolved.unwrap_or(false))
        .map_err(selector_query_error("external_code_callers failed"))?;
    json_value(result)
}

fn handle_code_callers_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let _args: CodeCallersArgs = parse_args(args)?;
    json_value(query::CallerResult {
        callers: Vec::new(),
        counts_by_kind: query::CountsByKind::default(),
        unresolved_sample: Vec::new(),
    })
}

fn handle_code_callees(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: CodeCalleesArgs = parse_args(args)?;
    let selector = normalize_selector(&args.selector, args.source(), catalog)?;
    let result = query::find_callees(db, &selector, args.include_unresolved.unwrap_or(false))
        .map_err(selector_query_error("external_code_callees failed"))?;
    json_value(result)
}

fn handle_code_callees_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let _args: CodeCalleesArgs = parse_args(args)?;
    json_value(query::CalleeResult {
        callees: Vec::new(),
        counts_by_kind: query::CountsByKind::default(),
        unresolved_sample: Vec::new(),
    })
}

fn handle_knowledge_context(
    args: &Value,
    db: &Connection,
    catalog: &CatalogResolver,
) -> Result<Value, McpHandlerError> {
    let args: KnowledgeContextArgs = parse_args(args)?;
    args.validate()?;
    let source = args.source();
    let resolved = resolve_revision(catalog, source, &args.package, args.revision_ref())?;
    let result = knowledge::query_knowledge_context(
        db,
        &KnowledgeContextOptions {
            query: args.query,
            source: resolved.source,
            package: resolved.package,
            revision: resolved.revision,
            scope: args.scope.unwrap_or(KnowledgeScope::All),
            limit: args.limit.unwrap_or(8),
            query_vec: args.query_vec,
        },
    )
    .map_err(internal_error("external_knowledge_context failed"))?;
    json_value(result)
}

fn handle_knowledge_context_without_catalog(args: &Value) -> Result<Value, McpHandlerError> {
    let args: KnowledgeContextArgs = parse_args(args)?;
    args.validate()?;
    json_value(knowledge::KnowledgeContextResult {
        primary_evidence: Vec::new(),
        supporting_docs: Vec::new(),
        confidence: "low".to_owned(),
        answerable: false,
        catalog_generation: None,
        graph_content_hash: None,
        candidates: knowledge::KnowledgeCandidateSummary {
            total: 0,
            returned_primary: 0,
            returned_supporting_docs: 0,
            total_code: 0,
            total_docs: 0,
        },
    })
}

fn handle_index_requires_lambda(args: &Value) -> Result<Value, McpHandlerError> {
    let args: ExternalIndexArgs = parse_args(args)?;
    args.validate()?;
    Err(McpHandlerError::Internal(
        "external_index requires Lambda routing with a Step Functions client".to_owned(),
    ))
}

fn handle_index_status_requires_lambda(args: &Value) -> Result<Value, McpHandlerError> {
    let args: ExternalIndexStatusArgs = parse_args(args)?;
    args.validate()?;
    Ok(json!({
        "status": "failed",
        "job_id": args.job_id,
        "error": {
            "code": "lambda_routing_required",
            "detail": "external_index_status requires Lambda routing with a job store"
        }
    }))
}

#[expect(
    clippy::future_not_send,
    reason = "handler borrows a DuckDB connection, which is intentionally not Sync"
)]
pub async fn route_index(
    args: &Value,
    _db: &Connection,
    catalog: &CatalogResolver,
    jobs: &dyn JobStore,
    sfn_client: &impl IndexExecutionStarter,
    caller_id: &str,
) -> Result<Value, McpHandlerError> {
    if let Some(response) = route_index_warm_lookup(args, catalog)? {
        return Ok(response);
    }
    route_index_inner(args, jobs, sfn_client, caller_id).await
}

pub async fn route_index_without_catalog(
    args: &Value,
    jobs: &dyn JobStore,
    sfn_client: &impl IndexExecutionStarter,
    caller_id: &str,
) -> Result<Value, McpHandlerError> {
    route_index_inner(args, jobs, sfn_client, caller_id).await
}

/// Performs the synchronous catalog portion of `external_index` routing.
///
/// `Some` contains a terminal validation rejection or warm catalog hit. `None`
/// means routing must continue through [`route_index_without_catalog`], which
/// performs DNS, rate-limit, and enqueue I/O without borrowing DuckDB.
pub fn route_index_warm_lookup(
    args: &Value,
    catalog: &CatalogResolver,
) -> Result<Option<Value>, McpHandlerError> {
    let args: ExternalIndexArgs = parse_args(args)?;
    args.validate()?;

    let validate_options = index_validate_options();
    let parsed_url = match abuse::validate(args.source_url.trim(), &validate_options) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(Some(json!({
                "status": "rejected",
                "reason": format!("source_url: {error}")
            })));
        }
    };

    let identity = match ExternalIndexIdentity::canonicalize(&args, &parsed_url) {
        Ok(identity) => identity,
        Err(reason) => {
            return Ok(Some(json!({
                "status": "rejected",
                "reason": reason
            })));
        }
    };

    if args.force.unwrap_or(false) {
        return Ok(None);
    }
    if let Some(resolved) = lookup_complete_catalog_revision(
        catalog,
        &identity.source,
        &identity.package,
        &identity.revision,
    )? {
        return Ok(Some(json!({
            "status": "complete",
            "snapshot_id": resolved.snapshot_id,
            "revision": resolved.revision
        })));
    }
    if let Some(legacy_source) = identity.legacy_warm_source.as_deref() {
        if let Some(resolved) = lookup_complete_catalog_revision(
            catalog,
            legacy_source,
            &identity.package,
            &identity.revision,
        )? {
            return Ok(Some(json!({
                "status": "complete",
                "snapshot_id": resolved.snapshot_id,
                "revision": resolved.revision
            })));
        }
    }
    Ok(None)
}

async fn route_index_inner(
    args: &Value,
    jobs: &dyn JobStore,
    _sfn_client: &impl IndexExecutionStarter,
    caller_id: &str,
) -> Result<Value, McpHandlerError> {
    let args: ExternalIndexArgs = parse_args(args)?;
    args.validate()?;

    let validate_options = index_validate_options();
    let parsed_url = match abuse::validate(args.source_url.trim(), &validate_options) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(json!({
                "status": "rejected",
                "reason": format!("source_url: {error}")
            }));
        }
    };

    let identity = match ExternalIndexIdentity::canonicalize(&args, &parsed_url) {
        Ok(identity) => identity,
        Err(reason) => {
            return Ok(json!({
                "status": "rejected",
                "reason": reason
            }));
        }
    };

    if let Err(error) = abuse::resolve_and_check_dns(&parsed_url) {
        return Ok(json!({
            "status": "rejected",
            "reason": format!("source_url: {error}")
        }));
    }

    // Preserve the existing per-owner request rate limit for every request
    // that can enqueue, including force=true and active-dedupe repeats.
    match jobs
        .check_index_rate_limit(caller_id, index_rate_limit_per_minute())
        .await
    {
        Ok(()) => {}
        Err(JobsError::RateLimited) => {
            return Ok(json!({
                "status": "rejected",
                "reason": "rate_limit",
                "retry_after_seconds": RATE_LIMIT_RETRY_AFTER_SECONDS
            }));
        }
        Err(error) => return Err(jobs_error("external_index rate limit failed")(error)),
    }

    // ─── Bounded enqueue admission ─────────────────────────────────────────
    //
    // Build the BacklogOwner from the authenticated caller context. The first
    // implementation maps to BacklogOwnerKind::Caller using the existing
    // caller_id; future deployments can scope backlogs by User or TenantUser
    // without changing the queue/drainer logic because DynamoDB keys are
    // derived from (kind, id).
    let owner = backlog_owner_from_caller(caller_id);
    let config = index_queue_config();

    let request = CreateJobRequest {
        source: identity.source,
        package: identity.package,
        revision: identity.revision,
        source_url: identity.source_url,
        source_url_hash: identity.source_url_hash,
        source_kind: source_kind_label(identity.source_kind).to_owned(),
        caller_id: caller_id.to_owned(),
    };

    let mut enqueue_attempt = 0;
    let enqueue_result = loop {
        match jobs
            .enqueue_job(request.clone(), owner.clone(), &config)
            .await
        {
            Err(JobsError::Conflict) if enqueue_attempt < ENQUEUE_CONFLICT_MAX_RETRIES => {
                tokio::time::sleep(enqueue_conflict_backoff(enqueue_attempt)).await;
                enqueue_attempt += 1;
            }
            result => break result,
        }
    };

    match enqueue_result {
        Ok(EnqueueOutcome::Enqueued(record)) | Ok(EnqueueOutcome::Existing(record)) => {
            Ok(queued_job_response(&record))
        }
        Err(JobsError::QueueFull) => Ok(json!({
            "status": "rejected",
            "reason": "queue_full",
            "max_queued_jobs_per_owner": config.max_queued_per_owner,
            "retry_after_seconds": RATE_LIMIT_RETRY_AFTER_SECONDS
        })),
        Err(JobsError::GlobalQueueFull) => Ok(json!({
            "status": "rejected",
            "reason": "global_queue_full",
            "max_queued_jobs_global": config.max_queued_global,
            "retry_after_seconds": RATE_LIMIT_RETRY_AFTER_SECONDS
        })),
        // TransactionConflict / concurrent write contention: the cap may have
        // room. Surface as a structured retryable rejection, NOT quota-full.
        Err(JobsError::Conflict) => Ok(json!({
            "status": "rejected",
            "reason": "conflict",
            "retry_after_seconds": 5
        })),
        Err(error) => Err(jobs_error("external_index enqueue failed")(error)),
    }
}

fn enqueue_conflict_backoff(attempt: u32) -> Duration {
    let exponential_ms = ENQUEUE_CONFLICT_BACKOFF_BASE_MS
        .saturating_mul(1_u64 << attempt.min(10))
        .min(ENQUEUE_CONFLICT_BACKOFF_MAX_MS);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::from(duration.subsec_nanos()) % ENQUEUE_CONFLICT_BACKOFF_BASE_MS)
        .unwrap_or_default();
    Duration::from_millis(exponential_ms.saturating_add(jitter_ms))
}

pub async fn route_index_status(
    args: &Value,
    jobs: &dyn JobStore,
    checker: Option<&dyn ExecutionStatusChecker>,
) -> Result<Value, McpHandlerError> {
    route_index_status_inner(args, jobs, checker, None).await
}

pub async fn route_index_status_for_caller(
    args: &Value,
    jobs: &dyn JobStore,
    checker: Option<&dyn ExecutionStatusChecker>,
    caller_id: &str,
) -> Result<Value, McpHandlerError> {
    route_index_status_inner(args, jobs, checker, Some(caller_id)).await
}

async fn route_index_status_inner(
    args: &Value,
    jobs: &dyn JobStore,
    checker: Option<&dyn ExecutionStatusChecker>,
    caller_id: Option<&str>,
) -> Result<Value, McpHandlerError> {
    let args: ExternalIndexStatusArgs = parse_args(args)?;
    args.validate()?;
    let Some(record) = jobs
        .lookup_job(&args.job_id)
        .await
        .map_err(jobs_error("external_index_status lookup failed"))?
    else {
        return Ok(json!({ "status": "not_found" }));
    };

    if caller_id.is_some_and(|caller_id| record.caller_id != caller_id) {
        return Ok(json!({ "status": "not_found" }));
    }

    let record = update_stale_job(record, jobs, checker).await?;
    Ok(index_status_response(&record))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogArgs {
    source: Option<String>,
    package: Option<String>,
    revision: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    path: Option<String>,
    name_filter: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

impl CatalogArgs {
    fn validate(&self) -> Result<(), McpHandlerError> {
        if let Some(source) = &self.source {
            validate_non_empty("source", source)?;
        }
        if let Some(package) = &self.package {
            validate_non_empty("package", package)?;
        }
        if let Some(revision) = &self.revision {
            validate_non_empty("revision", revision)?;
        }
        if let Some(ref_name) = &self.ref_name {
            validate_non_empty("ref", ref_name)?;
        }
        validate_revision_choice(self.revision.as_deref(), self.ref_name.as_deref())
    }

    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }

    fn revision_ref(&self) -> Option<&str> {
        self.revision.as_deref().or(self.ref_name.as_deref())
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(50).clamp(1, 200)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeSearchArgs {
    query: String,
    package: String,
    source: Option<String>,
    revision: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    symbol_kind: Option<String>,
    limit: Option<usize>,
}

impl CodeSearchArgs {
    fn validate(&self) -> Result<(), McpHandlerError> {
        validate_non_empty("query", &self.query)?;
        validate_non_empty("package", &self.package)?;
        validate_revision_choice(self.revision.as_deref(), self.ref_name.as_deref())
    }

    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }

    fn revision_ref(&self) -> Option<&str> {
        self.revision.as_deref().or(self.ref_name.as_deref())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeReadArgs {
    selector: String,
    source: Option<String>,
    context_lines: Option<usize>,
}

impl CodeReadArgs {
    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeCallersArgs {
    selector: String,
    source: Option<String>,
    include_unresolved: Option<bool>,
}

impl CodeCallersArgs {
    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeCalleesArgs {
    selector: String,
    source: Option<String>,
    include_unresolved: Option<bool>,
}

impl CodeCalleesArgs {
    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnowledgeContextArgs {
    query: String,
    package: String,
    source: Option<String>,
    revision: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    scope: Option<KnowledgeScope>,
    limit: Option<usize>,
    query_vec: Option<Vec<f32>>,
}

impl KnowledgeContextArgs {
    fn validate(&self) -> Result<(), McpHandlerError> {
        validate_non_empty("query", &self.query)?;
        validate_non_empty("package", &self.package)?;
        validate_revision_choice(self.revision.as_deref(), self.ref_name.as_deref())?;
        validate_query_vec(self.query_vec.as_deref())
    }

    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
    }

    fn revision_ref(&self) -> Option<&str> {
        self.revision.as_deref().or(self.ref_name.as_deref())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalIndexArgs {
    package: String,
    revision: String,
    source_url: String,
    source_kind: Option<ExternalIndexSourceKind>,
    source: Option<String>,
    force: Option<bool>,
}

impl ExternalIndexArgs {
    fn validate(&self) -> Result<(), McpHandlerError> {
        validate_non_empty("package", &self.package)?;
        validate_non_empty("revision", &self.revision)?;
        validate_non_empty("source_url", &self.source_url)?;
        if let Some(source) = self.source.as_deref() {
            validate_non_empty("source", source)?;
        }
        Ok(())
    }

    fn source_kind(&self, inferred: SourceKind) -> SourceKind {
        match self.source_kind {
            Some(ExternalIndexSourceKind::Git) => SourceKind::Git,
            Some(ExternalIndexSourceKind::Tarball) => SourceKind::Tarball,
            None => inferred,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalIndexIdentity {
    source: String,
    legacy_warm_source: Option<String>,
    package: String,
    revision: String,
    source_url: String,
    source_url_hash: String,
    source_kind: SourceKind,
}

impl ExternalIndexIdentity {
    fn canonicalize(
        args: &ExternalIndexArgs,
        parsed_url: &abuse::ParsedSourceUrl,
    ) -> Result<Self, &'static str> {
        let package = args.package.trim();
        let revision = args.revision.trim();
        let source_url = args.source_url.trim();
        let source = args.source.as_deref().map(str::trim);

        let (source, legacy_warm_source, package, source_url, source_kind) =
            if let Some((url_package, url_revision)) =
                crates_io_download_coordinates(source_url, &parsed_url.hostname)
            {
                let package = normalize_crates_io_package(package);
                let fetch_package = url_package.to_ascii_lowercase();
                let normalized_url_package = normalize_crates_io_package(url_package);
                if package != normalized_url_package {
                    return Err("source_url_package_mismatch");
                }
                if revision != url_revision {
                    return Err("source_url_revision_mismatch");
                }

                let source = match source {
                    None => DEFAULT_SOURCE.to_owned(),
                    Some(value) if is_crates_io_source_alias(value) => DEFAULT_SOURCE.to_owned(),
                    Some(value) => value.to_owned(),
                };
                let legacy_warm_source = args
                    .source
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| is_legacy_crates_io_source_alias(value))
                    .map(str::to_owned);
                let canonical_url =
                    format!("https://crates.io/api/v1/crates/{fetch_package}/{revision}/download");
                (
                    source,
                    legacy_warm_source,
                    package,
                    canonical_url,
                    SourceKind::Tarball,
                )
            } else {
                (
                    source.unwrap_or(DEFAULT_INDEX_SOURCE).to_owned(),
                    None,
                    package.to_owned(),
                    source_url.to_owned(),
                    args.source_kind(parsed_url.source_kind),
                )
            };
        let source_url_hash = source_url_hash(&source_url);

        Ok(Self {
            source,
            legacy_warm_source,
            package,
            revision: revision.to_owned(),
            source_url,
            source_url_hash,
            source_kind,
        })
    }
}

fn crates_io_download_coordinates<'a>(
    source_url: &'a str,
    hostname: &str,
) -> Option<(&'a str, &'a str)> {
    if hostname != "crates.io" {
        return None;
    }

    let (scheme, rest) = source_url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let authority_host = authority.strip_suffix(":443").unwrap_or(authority);
    if !authority_host.eq_ignore_ascii_case("crates.io")
        && !authority_host.eq_ignore_ascii_case("crates.io.")
    {
        return None;
    }
    let path = rest[authority_end..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let segments = path.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["", "api", "v1", "crates", package, revision, "download"]
        | ["", "api", "v1", "crates", package, revision, "download", ""]
            if !package.is_empty() && !revision.is_empty() =>
        {
            Some((package, revision))
        }
        _ => None,
    }
}

fn normalize_crates_io_package(package: &str) -> String {
    package.trim().to_ascii_lowercase().replace('_', "-")
}

fn is_crates_io_source_alias(source: &str) -> bool {
    ["crates.io", "crates-io", DEFAULT_SOURCE]
        .iter()
        .any(|alias| source.eq_ignore_ascii_case(alias))
}

fn is_legacy_crates_io_source_alias(source: &str) -> bool {
    ["crates.io", "crates-io"]
        .iter()
        .any(|alias| source.eq_ignore_ascii_case(alias))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExternalIndexSourceKind {
    Git,
    Tarball,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalIndexStatusArgs {
    job_id: String,
}

impl ExternalIndexStatusArgs {
    fn validate(&self) -> Result<(), McpHandlerError> {
        validate_non_empty("job_id", &self.job_id)
    }
}

#[derive(Debug)]
struct ParsedExternalSelector {
    package: String,
    revision_or_ref: Option<String>,
    qualified_name: String,
}

fn parse_args<T>(args: &Value) -> Result<T, McpHandlerError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(args.clone()).map_err(|error| {
        McpHandlerError::InvalidParams(format!("failed to parse tool arguments: {error}"))
    })
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), McpHandlerError> {
    if value.trim().is_empty() {
        return Err(McpHandlerError::InvalidParams(format!(
            "field '{field}' must be non-empty"
        )));
    }
    Ok(())
}

fn validate_revision_choice(
    revision: Option<&str>,
    ref_name: Option<&str>,
) -> Result<(), McpHandlerError> {
    if revision.is_some() && ref_name.is_some() {
        return Err(McpHandlerError::InvalidParams(
            "use either 'revision' or 'ref', not both".to_owned(),
        ));
    }
    Ok(())
}

fn validate_query_vec(query_vec: Option<&[f32]>) -> Result<(), McpHandlerError> {
    let Some(query_vec) = query_vec else {
        return Ok(());
    };

    if query_vec.len() != KNOWLEDGE_QUERY_VECTOR_DIMENSIONS {
        return Err(McpHandlerError::InvalidParams(format!(
            "field 'query_vec' must contain {KNOWLEDGE_QUERY_VECTOR_DIMENSIONS} floats"
        )));
    }
    if query_vec.iter().any(|value| !value.is_finite()) {
        return Err(McpHandlerError::InvalidParams(
            "field 'query_vec' must contain only finite floats".to_owned(),
        ));
    }
    Ok(())
}

fn resolve_revision(
    catalog: &CatalogResolver,
    source: &str,
    package: &str,
    revision_or_ref: Option<&str>,
) -> Result<ResolvedRevision, McpHandlerError> {
    let revision_or_ref = revision_or_ref.unwrap_or(DEFAULT_REF);
    catalog
        .resolve(source, package, revision_or_ref)
        .map_err(catalog_error(format!(
            "{source}/{package}@{revision_or_ref}"
        )))
}

fn lookup_complete_catalog_revision(
    catalog: &CatalogResolver,
    source: &str,
    package: &str,
    revision: &str,
) -> Result<Option<ResolvedRevision>, McpHandlerError> {
    let resolved = match catalog.resolve(source, package, revision) {
        Ok(resolved) => resolved,
        Err(error) => {
            let message = format!("{error:#}");
            if message.contains("not found") {
                return Ok(None);
            }
            return Err(McpHandlerError::Internal(format!(
                "{source}/{package}@{revision}: {message}"
            )));
        }
    };

    let package_catalog = readable_table(catalog.connection(), "package_catalog")
        .map_err(|error| McpHandlerError::Internal(format!("{error:#}")))?;
    let sql = format!(
        r"
        SELECT index_status
        FROM {package_catalog}
        WHERE source = ? AND package = ? AND revision = ?
        LIMIT 1
        "
    );
    let status: Option<String> = optional_no_rows(
        catalog.connection().query_row(
            &sql,
            params![source, package, resolved.revision.as_str()],
            |row| row.get(0),
        ),
        "external_index catalog status lookup failed",
    )?;

    if status.as_deref() == Some("complete") {
        Ok(Some(resolved))
    } else {
        Ok(None)
    }
}

fn normalize_selector(
    selector: &str,
    source: &str,
    catalog: &CatalogResolver,
) -> Result<String, McpHandlerError> {
    let selector = selector.trim();
    if selector.starts_with("pkg-symbol://") {
        return Ok(selector.to_owned());
    }
    let parsed = parse_external_selector(selector)?;
    let resolved = resolve_revision(
        catalog,
        source,
        &parsed.package,
        parsed.revision_or_ref.as_deref(),
    )?;
    Ok(format!(
        "pkg:{}|{}@{}::{}",
        resolved.source, resolved.package, resolved.revision, parsed.qualified_name
    ))
}

fn parse_external_selector(selector: &str) -> Result<ParsedExternalSelector, McpHandlerError> {
    let trimmed = selector.trim();
    let Some(selector_body) = trimmed.strip_prefix("pkg:") else {
        return Err(McpHandlerError::InvalidParams(format!(
            "external selector must start with 'pkg:': {selector}"
        )));
    };
    let Some((package_revision, qualified_name)) = selector_body.split_once("::") else {
        return Err(McpHandlerError::InvalidParams(format!(
            "external selector must include a package and symbol path: {selector}"
        )));
    };
    if qualified_name.is_empty() {
        return Err(McpHandlerError::InvalidParams(format!(
            "external selector must include a symbol path: {selector}"
        )));
    }

    let (package, revision_or_ref) = match package_revision.split_once('@') {
        Some((package, revision_or_ref)) if !package.is_empty() && !revision_or_ref.is_empty() => {
            (package.to_owned(), Some(revision_or_ref.to_owned()))
        }
        Some(_) => {
            return Err(McpHandlerError::InvalidParams(format!(
                "external selector has an invalid package revision: {selector}"
            )))
        }
        None if !package_revision.is_empty() => (package_revision.to_owned(), None),
        None => {
            return Err(McpHandlerError::InvalidParams(format!(
                "external selector must include a package: {selector}"
            )))
        }
    };

    Ok(ParsedExternalSelector {
        package,
        revision_or_ref,
        qualified_name: qualified_name.to_owned(),
    })
}

fn catalog_error(target: String) -> impl FnOnce(anyhow::Error) -> McpHandlerError {
    move |error| {
        let message = format!("{error:#}");
        if message.contains("not found") {
            McpHandlerError::NotFound(format!("{target}: {message}"))
        } else {
            McpHandlerError::Internal(format!("{target}: {message}"))
        }
    }
}

fn internal_error(context: &'static str) -> impl FnOnce(anyhow::Error) -> McpHandlerError {
    move |error| McpHandlerError::Internal(format!("{context}: {error:#}"))
}

fn selector_query_error(context: &'static str) -> impl FnOnce(anyhow::Error) -> McpHandlerError {
    move |error| {
        if let Some(ambiguity) = error.downcast_ref::<query::AmbiguousSelectorError>() {
            let candidates = ambiguity
                .candidates
                .iter()
                .map(|candidate| candidate.uri.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            McpHandlerError::InvalidParams(format!(
                "{context}: {ambiguity}; candidates: {candidates}"
            ))
        } else {
            McpHandlerError::Internal(format!("{context}: {error:#}"))
        }
    }
}

fn jobs_error(context: &'static str) -> impl FnOnce(JobsError) -> McpHandlerError {
    move |error| match error {
        JobsError::NotFound => McpHandlerError::NotFound(format!("{context}: {error}")),
        JobsError::Conflict => McpHandlerError::Internal(format!("{context}: {error}")),
        JobsError::ConcurrentLimit => McpHandlerError::Internal(format!("{context}: {error}")),
        JobsError::RateLimited => McpHandlerError::Internal(format!("{context}: {error}")),
        // The admission path handles QueueFull / GlobalQueueFull explicitly
        // before reaching this mapper. This branch is for other callers (e.g.
        // route_index_status) where a queue-full error is unexpected and maps
        // to Internal.
        JobsError::QueueFull | JobsError::GlobalQueueFull | JobsError::GlobalRunningFull => {
            McpHandlerError::Internal(format!("{context}: {error}"))
        }
        JobsError::Db(error) => McpHandlerError::Internal(format!("{context}: {error}")),
    }
}

fn optional_no_rows<T>(
    result: duckdb::Result<T>,
    context: &'static str,
) -> Result<Option<T>, McpHandlerError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(McpHandlerError::Internal(format!("{context}: {error}"))),
    }
}

fn json_value<T>(value: T) -> Result<Value, McpHandlerError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        McpHandlerError::Internal(format!("failed to serialize response: {error}"))
    })
}

pub(crate) async fn update_stale_job(
    record: JobRecord,
    jobs: &dyn JobStore,
    checker: Option<&dyn ExecutionStatusChecker>,
) -> Result<JobRecord, McpHandlerError> {
    // ─── Terminal-with-leftover-token repair ────────────────────────────
    //
    // A live worker may record terminal status and then fail the running-quota
    // release with a transient `TransactionConflict`. The release error is
    // logged best-effort by the worker; this poll is the durable retry path.
    // When a poll finds a terminal job that still holds a `RUNNING#` token,
    // release it exactly once. The release is idempotent: if the token is
    // already gone the call is a no-op, and a `Conflict` surfaces so the next
    // poll retries rather than silently leaking capacity.
    if record.status.is_terminal_for_quota() {
        jobs.release_running_quota(&record)
            .await
            .map_err(jobs_error(
                "external_index_status terminal quota release repair",
            ))?;
        return Ok(record);
    }

    // Include `Dispatching` so a job that was dispatched but whose worker never
    // reported its first stage (Step Functions failed before the first update)
    // can still be reconciled here. Without it the job would be stuck
    // indefinitely holding running capacity.
    if !matches!(
        record.status,
        JobStatus::Queued | JobStatus::Dispatching | JobStatus::Running
    ) {
        return Ok(record);
    }
    if !is_stale_job(&record) {
        return Ok(record);
    }

    let Some(checker) = checker else {
        return Ok(record);
    };
    let Some(execution_arn) = record.execution_arn.as_deref() else {
        return Ok(record);
    };

    let Ok(Some(outcome)) = checker.describe_execution(execution_arn) else {
        return Ok(record);
    };

    match outcome.status {
        ExecutionOutcomeStatus::Running => Ok(record),
        ExecutionOutcomeStatus::Succeeded => {
            let snapshot_id = outcome
                .output
                .as_ref()
                .and_then(|output| output.get("snapshot_id"))
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    McpHandlerError::Internal(
                        "DescribeExecution succeeded output missing snapshot_id".to_owned(),
                    )
                })?;
            let row_counts = outcome
                .output
                .as_ref()
                .and_then(|output| {
                    output
                        .get("rows_inserted")
                        .or_else(|| output.get("row_counts"))
                })
                .cloned()
                .unwrap_or_else(|| json!({}));

            // Use the combined method so running quota is released exactly once
            // after the terminal status is recorded. A stale running/dispatching
            // job repaired here would otherwise leak owner/global running
            // capacity.
            jobs.mark_complete_and_release_running_quota(&record.job_id, snapshot_id, row_counts)
                .await
                .map_err(jobs_error("external_index_status update complete failed"))
        }
        ExecutionOutcomeStatus::Failed => {
            let error = outcome
                .error
                .as_deref()
                .filter(|error| !error.trim().is_empty())
                .unwrap_or("execution: failed");
            let (code, detail) = split_job_error(error);
            jobs.mark_failed_and_release_running_quota(&record.job_id, code, detail)
                .await
                .map_err(jobs_error(
                    "external_index_status update failed status failed",
                ))
        }
    }
}

fn is_stale_job(record: &JobRecord) -> bool {
    updated_at_age(&record.updated_at).is_some_and(|age| age >= STALE_JOB_REPAIR_AFTER)
}

fn updated_at_age(updated_at: &str) -> Option<Duration> {
    let updated_millis = updated_at.parse::<u128>().ok()?;
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let elapsed = now_millis.saturating_sub(updated_millis);
    Some(Duration::from_millis(
        elapsed.min(u128::from(u64::MAX)) as u64
    ))
}

fn sfn_execution_outcome(
    output: aws_sdk_sfn::operation::describe_execution::DescribeExecutionOutput,
) -> Result<ExecutionOutcome, McpHandlerError> {
    let status = match output.status() {
        AwsExecutionStatus::Succeeded => ExecutionOutcomeStatus::Succeeded,
        AwsExecutionStatus::Running | AwsExecutionStatus::PendingRedrive => {
            ExecutionOutcomeStatus::Running
        }
        AwsExecutionStatus::Failed | AwsExecutionStatus::Aborted | AwsExecutionStatus::TimedOut => {
            ExecutionOutcomeStatus::Failed
        }
        other => match other.as_str() {
            "SUCCEEDED" => ExecutionOutcomeStatus::Succeeded,
            "RUNNING" | "PENDING_REDRIVE" => ExecutionOutcomeStatus::Running,
            _ => ExecutionOutcomeStatus::Failed,
        },
    };
    let output_value = output
        .output()
        .map(|raw| {
            serde_json::from_str(raw).map_err(|error| {
                McpHandlerError::Internal(format!("DescribeExecution output JSON: {error}"))
            })
        })
        .transpose()?;
    let error = execution_error_message(&status, &output);

    Ok(ExecutionOutcome {
        status,
        output: output_value,
        error,
    })
}

fn execution_error_message(
    status: &ExecutionOutcomeStatus,
    output: &aws_sdk_sfn::operation::describe_execution::DescribeExecutionOutput,
) -> Option<String> {
    match (output.error(), output.cause()) {
        (Some(error), Some(cause)) if !cause.trim().is_empty() => Some(format!("{error}: {cause}")),
        (Some(error), _) => Some(error.to_owned()),
        (None, Some(cause)) if !cause.trim().is_empty() => Some(format!("execution: {cause}")),
        (None, _) if *status == ExecutionOutcomeStatus::Failed => {
            Some(format!("execution: {}", output.status().as_str()))
        }
        _ => None,
    }
}

fn source_url_hash(source_url: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in source_url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn source_kind_label(source_kind: SourceKind) -> &'static str {
    match source_kind {
        SourceKind::Git => "git",
        SourceKind::Tarball => "tarball",
    }
}

/// Whether the Step Functions worker should prefetch the source before
/// building. Used by the drainer to construct the dispatch payload from the
/// queued `JobRecord`.
pub fn should_prefetch_source(source_kind: SourceKind, hostname: &str) -> bool {
    match source_kind {
        SourceKind::Git => true,
        SourceKind::Tarball => !is_s3_https_hostname(hostname),
    }
}

pub fn is_s3_https_hostname(hostname: &str) -> bool {
    let Some(prefix) = hostname.strip_suffix(".amazonaws.com") else {
        return false;
    };

    if prefix == "s3" {
        return true;
    }

    if let Some(region) = prefix.strip_prefix("s3.") {
        return is_single_label(region);
    }

    if let Some(bucket) = prefix.strip_suffix(".s3") {
        return !bucket.is_empty();
    }

    prefix
        .rsplit_once(".s3.")
        .is_some_and(|(bucket, region)| !bucket.is_empty() && is_single_label(region))
}

pub fn is_single_label(value: &str) -> bool {
    !value.is_empty() && !value.contains('.')
}

fn index_validate_options() -> ValidateOptions {
    ValidateOptions {
        tarball_size_cap_bytes: env_u64(
            "SPUR_CONTEXT_MAX_TARBALL_BYTES",
            DEFAULT_TARBALL_SIZE_CAP_BYTES,
        ),
        git_size_cap_bytes: env_u64("SPUR_CONTEXT_MAX_GIT_BYTES", DEFAULT_GIT_SIZE_CAP_BYTES),
        allowed_domains: env::var("SPUR_CONTEXT_ALLOWED_SOURCE_DOMAINS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Resource limits for the Step Functions index worker payload. Used by the
/// drainer to construct the dispatch payload from the queued `JobRecord`.
pub fn index_resource_limits(source_kind: SourceKind) -> IndexResourceLimits {
    let max_source_bytes = match source_kind {
        SourceKind::Git => env_u64("SPUR_CONTEXT_MAX_GIT_BYTES", DEFAULT_GIT_SIZE_CAP_BYTES),
        SourceKind::Tarball => env_u64(
            "SPUR_CONTEXT_MAX_TARBALL_BYTES",
            DEFAULT_TARBALL_SIZE_CAP_BYTES,
        ),
    };
    IndexResourceLimits {
        max_source_bytes,
        max_build_seconds: env_u64("SPUR_CONTEXT_MAX_BUILD_SECONDS", DEFAULT_MAX_BUILD_SECONDS),
    }
}

/// Build the [`IndexExecutionRequest`] payload for a dispatched job from its
/// persisted [`JobRecord`].
///
/// This reconstructs the exact payload contract the old immediate-start
/// admission path used to build inline:
///
/// ```json
/// {
///   "job_id": "<job_id>",
///   "source": "<source>",
///   "package": "<package>",
///   "revision": "<revision>",
///   "source_url": "<source_url>",
///   "source_kind": "<git|tarball>",
///   "prefetch_source": <bool>,
///   "caller_id": "<caller_id>",
///   "limits": { "max_source_bytes": <u64>, "max_build_seconds": <u64> }
/// }
/// ```
///
/// The drainer calls this after the transactional queued→dispatching
/// transition succeeds, then hands the resulting [`IndexExecutionRequest`] to
/// the production Step Functions starter. The `name` is the `job_id` so Step
/// Functions execution names deduplicate dispatch (a retry for the same job
/// lands on the same execution name).
///
/// `prefetch_source` and `limits` are derived from the stored `source_url` /
/// `source_kind` using the same helpers as the old admission path. The URL was
/// already validated at enqueue time; if re-parsing the hostname fails
/// unexpectedly (e.g. env-var-driven allowlist drift), `prefetch_source`
/// defaults to `true` — a safe, conservative choice that never silently drops
/// a dispatch.
pub fn build_index_execution_request(record: &JobRecord) -> IndexExecutionRequest {
    let source_kind = parse_source_kind(&record.source_kind);
    let hostname = hostname_from_url(&record.source_url);
    let prefetch_source = should_prefetch_source(source_kind, &hostname);
    let limits = index_resource_limits(source_kind);
    let input = json!({
        "job_id": record.job_id,
        "source": record.source,
        "package": record.package,
        "revision": record.revision,
        "source_url": record.source_url,
        "source_kind": record.source_kind,
        "prefetch_source": prefetch_source,
        "caller_id": record.caller_id,
        "limits": {
            "max_source_bytes": limits.max_source_bytes,
            "max_build_seconds": limits.max_build_seconds
        }
    });
    IndexExecutionRequest {
        name: record.job_id.clone(),
        input,
    }
}

/// Parse a `source_kind` label string ("git" / "tarball") back into the typed
/// enum. Defaults to [`SourceKind::Git`] for unrecognized labels (git is the
/// safer default — it always prefetches).
fn parse_source_kind(label: &str) -> SourceKind {
    match label.trim() {
        "tarball" => SourceKind::Tarball,
        _ => SourceKind::Git,
    }
}

/// Extract the lower-cased hostname from a URL string for the prefetch-source
/// decision. This is a lightweight parse (no DNS, no validation) — the URL was
/// already validated at enqueue time. Returns an empty string if the hostname
/// cannot be extracted, in which case `should_prefetch_source` defaults to
/// `true`.
fn hostname_from_url(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = authority.split(':').next().unwrap_or(authority);
    let host = host.strip_prefix('@').unwrap_or(host).trim().to_lowercase();
    if host.is_empty() {
        // Fallback: try the whole URL lower-cased so the S3-hostname check has
        // something to work with.
        url.trim().to_lowercase()
    } else {
        host
    }
}

fn index_rate_limit_per_minute() -> u32 {
    env_u32("SPUR_INDEX_RATE_LIMIT_PER_MINUTE", DEFAULT_CALLS_PER_MINUTE)
}

fn index_max_concurrent_jobs_per_caller() -> u32 {
    env_u32(
        "SPUR_INDEX_MAX_CONCURRENT_JOBS_PER_CALLER",
        DEFAULT_MAX_CONCURRENT_JOBS_PER_CALLER,
    )
}

pub const DEFAULT_INDEX_DRAINER_BATCH_LIMIT: usize = 8;
pub const DEFAULT_INDEX_DRAINER_SCAN_LIMIT_PER_SHARD: usize = 32;
pub const DEFAULT_INDEX_DRAINER_SCHEDULE_RATE_MINUTES: u64 = 1;
pub const MAX_INDEX_GLOBAL_RUNNING_TOKENS: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexDrainerLimits {
    pub max_dispatches_per_run: usize,
    pub scan_limit_per_shard: usize,
    pub rotation_interval_secs: u64,
}

/// Default per-owner queued backlog cap when the env var is not set. A non-zero
/// default enables the bounded-queue admission path out of the box; deployments
/// that want legacy reject-over-cap behavior set
/// `SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER=0`.
const DEFAULT_MAX_QUEUED_JOBS_PER_OWNER: u32 = 20;
const DEFAULT_QUEUE_SHARD_COUNT: u32 = 16;

/// Build the [`QueueConfig`] for the enqueue admission path from environment
/// variables. The dedicated per-owner running cap falls back to the legacy
/// `SPUR_INDEX_MAX_CONCURRENT_JOBS_PER_CALLER` value for deployments that have
/// not yet adopted the bounded-backlog configuration surface.
pub fn index_queue_config() -> QueueConfig {
    QueueConfig {
        max_queued_per_owner: env_u32(
            "SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER",
            DEFAULT_MAX_QUEUED_JOBS_PER_OWNER,
        ),
        max_queued_global: env_u32("SPUR_INDEX_MAX_QUEUED_JOBS_GLOBAL", 0),
        max_running_per_owner: env_u32(
            "SPUR_INDEX_MAX_RUNNING_JOBS_PER_OWNER",
            index_max_concurrent_jobs_per_caller(),
        ),
        max_running_global: env_u32("SPUR_INDEX_MAX_RUNNING_JOBS_GLOBAL", 0)
            .min(MAX_INDEX_GLOBAL_RUNNING_TOKENS),
        shard_count: env_u32("SPUR_INDEX_QUEUE_SHARD_COUNT", DEFAULT_QUEUE_SHARD_COUNT),
    }
}

pub fn index_drainer_limits() -> IndexDrainerLimits {
    IndexDrainerLimits {
        max_dispatches_per_run: env_usize(
            "SPUR_INDEX_DRAINER_BATCH_LIMIT",
            DEFAULT_INDEX_DRAINER_BATCH_LIMIT,
        )
        .max(1),
        scan_limit_per_shard: env_usize(
            "SPUR_INDEX_DRAINER_SCAN_LIMIT_PER_SHARD",
            DEFAULT_INDEX_DRAINER_SCAN_LIMIT_PER_SHARD,
        )
        .max(1),
        rotation_interval_secs: env_u64(
            "SPUR_INDEX_DRAINER_SCHEDULE_RATE_MINUTES",
            DEFAULT_INDEX_DRAINER_SCHEDULE_RATE_MINUTES,
        )
        .max(1)
        .saturating_mul(60),
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

/// Build a [`BacklogOwner`] from the authenticated caller context.
///
/// The first implementation maps all traffic to [`BacklogOwnerKind::Caller`]
/// using the existing `caller_id`. This is the stable owner identity that all
/// queue quota keys are derived from. When per-user backlog isolation is added
/// later, only this function needs to change — the enqueue/dispatch/drainer
/// logic derives everything from `(kind, id)`.
fn backlog_owner_from_caller(caller_id: &str) -> BacklogOwner {
    BacklogOwner::caller(caller_id)
}

fn queued_job_response(record: &JobRecord) -> Value {
    json!({
        "job_id": record.job_id,
        "status": record.status.to_string(),
        "execution_arn": record.execution_arn,
        "revision": record.revision
    })
}

fn index_status_response(record: &JobRecord) -> Value {
    let mut response = json!({
        "job_id": record.job_id,
        "status": record.status.to_string(),
        "revision": record.revision,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
        "attempt": record.attempt,
        "execution_arn": record.execution_arn
    });

    if let Some(stage) = record.stage.as_deref() {
        response["stage"] = json!(stage);
    }
    if let Some(snapshot_id) = record.snapshot_id {
        response["snapshot_id"] = json!(snapshot_id);
    }
    if let Some(row_counts) = record.row_counts.as_ref() {
        response["row_counts"] = row_counts.clone();
    }
    if record.status == JobStatus::Failed
        && (record.error_code.is_some() || record.error_detail.is_some())
    {
        response["error"] = job_error_response(
            record.error_code.as_deref().unwrap_or("execution"),
            record.error_detail.as_deref().unwrap_or("failed"),
        );
    }

    response
}

fn split_job_error(error: &str) -> (&str, &str) {
    error.split_once(':').map_or_else(
        || (error.trim(), ""),
        |(code, detail)| (code.trim(), detail.trim()),
    )
}

fn job_error_response(code: &str, detail: &str) -> Value {
    json!({
        "code": code,
        "detail": detail,
        "retriable": matches!(code, "fetch" | "commit" | "spot_interrupted" | "timeout")
    })
}

fn external_catalog_def() -> ToolDefinition {
    ToolDefinition {
        name: "external_catalog".to_owned(),
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
        name: "external_code_search".to_owned(),
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
        name: "external_code_read".to_owned(),
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
        name: "external_code_callers".to_owned(),
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
        name: "external_code_callees".to_owned(),
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
        name: "external_knowledge_context".to_owned(),
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
                    "description": "Optional precomputed NomicEmbedTextV15 query embedding produced with the `search_query:` prompt. When omitted, retrieval gracefully degrades to BM25-only."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_index_def() -> ToolDefinition {
    ToolDefinition {
        name: "external_index".to_owned(),
        description: "Queue on-demand indexing for a fetchable external package source.".to_owned(),
        input_schema: json!({
            "type": "object",
            "required": ["package", "revision", "source_url"],
            "properties": {
                "package": {
                    "type": "string",
                    "description": "Package name, for example serde or tokio. Surrounding whitespace is ignored; standard crates.io requests also lowercase the name and treat underscores as hyphens."
                },
                "revision": {
                    "type": "string",
                    "description": "Version, branch, tag, or SHA to index. Surrounding whitespace is ignored. For a standard crates.io download URL this version must match the URL."
                },
                "source_url": {
                    "type": "string",
                    "description": "Fetchable git or tarball URL for the source. Standard crates.io download URLs are validated against package/revision and stored as one canonical https://crates.io URL."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["git", "tarball"],
                    "description": "Optional source fetch strategy. When omitted it is inferred from source_url; standard crates.io download URLs always use tarball."
                },
                "source": {
                    "type": "string",
                    "description": "Catalog source namespace. For a standard crates.io download URL, omission and the aliases crates.io, crates-io, and registry:crates-io canonicalize to registry:crates-io. Other explicit namespaces are preserved after trimming; non-crates.io omission remains git:custom."
                },
                "force": {
                    "type": "boolean",
                    "default": false,
                    "description": "Bypass only the warm-path catalog hit check. Abuse and DNS checks, rate limits, active dedupe, and queue caps still apply."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn external_index_status_def() -> ToolDefinition {
    ToolDefinition {
        name: "external_index_status".to_owned(),
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
mod tests {
    use super::*;

    #[test]
    fn s3_https_hostname_detection_accepts_supported_forms() {
        for hostname in [
            "bucket.s3.amazonaws.com",
            "bucket.s3.us-east-1.amazonaws.com",
            "s3.amazonaws.com",
            "s3.us-east-1.amazonaws.com",
            "dotted.bucket.s3.us-east-1.amazonaws.com",
        ] {
            assert!(is_s3_https_hostname(hostname), "{hostname}");
        }
    }

    #[test]
    fn s3_https_hostname_detection_rejects_non_s3_hosts() {
        for hostname in [
            "example.com",
            "s3.example.com",
            "bucket.s3.amazonaws.com.evil.test",
            "s3.amazonaws.com.evil.test",
        ] {
            assert!(!is_s3_https_hostname(hostname), "{hostname}");
        }
    }

    #[test]
    fn prefetch_source_follows_source_kind_and_s3_tarball_rules() {
        assert!(should_prefetch_source(SourceKind::Git, "github.com"));
        assert!(should_prefetch_source(SourceKind::Tarball, "example.com"));
        assert!(!should_prefetch_source(
            SourceKind::Tarball,
            "bucket.s3.us-east-1.amazonaws.com"
        ));
    }
}
