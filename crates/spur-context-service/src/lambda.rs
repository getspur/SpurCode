//! AWS Lambda HTTP entry point for the context-service MCP surface.
//!
//! Serving intentionally reads only the published frozen DuckLake catalog
//! snapshot from S3 plus the S3 gold data files. It must not attach the live
//! ingest catalog backend, including Aurora/Postgres.
//!
//! The PoC measured roughly 15s cold starts from DuckDB import, extension
//! loading, and snapshot download, while warm invokes were fast. For
//! latency-sensitive serving, use provisioned concurrency, keep DuckDB
//! extensions baked into the Lambda package, and trim the package if init time
//! stays high.

use lambda_runtime::{Error, LambdaEvent};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Code,
    Knowledge,
}

pub fn tool_is_eligible(backend: BackendKind, tool: &str) -> bool {
    match backend {
        BackendKind::Code => matches!(
            tool,
            "external_catalog"
                | "external_index"
                | "external_index_status"
                | "external_code_search"
                | "external_code_read"
                | "external_code_callers"
                | "external_code_callees"
        ),
        BackendKind::Knowledge => matches!(tool, "external_knowledge_context"),
    }
}

pub(crate) fn control_route_is_eligible(
    backend: BackendKind,
    route: crate::auth::RequestRoute,
) -> bool {
    match backend {
        BackendKind::Code => matches!(
            route,
            crate::auth::RequestRoute::Discovery
                | crate::auth::RequestRoute::Login
                | crate::auth::RequestRoute::ApiKeyCreate
                | crate::auth::RequestRoute::ApiKeyList
                | crate::auth::RequestRoute::ApiKeyRevoke
                | crate::auth::RequestRoute::Legacy
                | crate::auth::RequestRoute::OAuth
                | crate::auth::RequestRoute::ApiKeyMcp
        ),
        BackendKind::Knowledge => matches!(
            route,
            crate::auth::RequestRoute::Legacy
                | crate::auth::RequestRoute::OAuth
                | crate::auth::RequestRoute::ApiKeyMcp
        ),
    }
}

#[cfg(test)]
static CODE_HANDLER_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static KNOWLEDGE_HANDLER_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub fn dispatch_to_serving_handler<T>(
    backend: BackendKind,
    handler: impl FnOnce(BackendKind) -> T,
) -> T {
    handler(backend)
}

pub async fn handler_for(backend: BackendKind, event: LambdaEvent<Value>) -> Result<Value, Error> {
    dispatch_to_serving_handler(backend, |selected| async move {
        match selected {
            BackendKind::Code => {
                #[cfg(feature = "code-lambda")]
                {
                    return code::handler(event).await;
                }
                #[cfg(not(feature = "code-lambda"))]
                {
                    Err(unavailable_backend("Code"))
                }
            }
            BackendKind::Knowledge => {
                #[cfg(feature = "knowledge-lambda")]
                {
                    return knowledge::handler(event).await;
                }
                #[cfg(not(feature = "knowledge-lambda"))]
                {
                    Err(unavailable_backend("Knowledge"))
                }
            }
        }
    })
    .await
}

fn unavailable_backend(name: &str) -> Error {
    std::io::Error::other(format!(
        "{name} Lambda backend is not enabled in this binary"
    ))
    .into()
}

#[cfg(all(test, feature = "code-lambda", feature = "knowledge-lambda"))]
mod handler_selection_tests {
    use std::sync::atomic::Ordering;

    use lambda_runtime::{Context, LambdaEvent};
    use serde_json::json;

    use super::{handler_for, BackendKind, CODE_HANDLER_CALLS, KNOWLEDGE_HANDLER_CALLS};

    fn event(tool: &str) -> LambdaEvent<serde_json::Value> {
        LambdaEvent::new(
            json!({
                "__spur_count_handler": true,
                "rawPath": "/mcp",
                "requestContext": { "http": { "method": "POST" } },
                "body": serde_json::to_string(&json!({ "tool": tool, "args": {} }))
                    .expect("tool body should serialize"),
                "isBase64Encoded": false
            }),
            Context::default(),
        )
    }

    #[tokio::test]
    async fn each_api_request_executes_exactly_one_selected_real_handler() {
        CODE_HANDLER_CALLS.store(0, Ordering::SeqCst);
        KNOWLEDGE_HANDLER_CALLS.store(0, Ordering::SeqCst);
        handler_for(BackendKind::Code, event("external_knowledge_context"))
            .await
            .expect("Code rejection should be a bounded proxy response");
        assert_eq!(CODE_HANDLER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(KNOWLEDGE_HANDLER_CALLS.load(Ordering::SeqCst), 0);

        CODE_HANDLER_CALLS.store(0, Ordering::SeqCst);
        KNOWLEDGE_HANDLER_CALLS.store(0, Ordering::SeqCst);
        handler_for(BackendKind::Knowledge, event("external_catalog"))
            .await
            .expect("Knowledge rejection should be a bounded proxy response");
        assert_eq!(CODE_HANDLER_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(KNOWLEDGE_HANDLER_CALLS.load(Ordering::SeqCst), 1);
    }
}

#[cfg(feature = "code-control")]
pub(crate) mod code_control {
    use std::env;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use aws_sdk_sfn::types::ExecutionStatus as AwsExecutionStatus;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::abuse::{self, SourceKind, ValidateOptions};
    use crate::code_backend::CodeBackend;
    use crate::jobs::{
        BacklogOwner, CreateJobRequest, EnqueueOutcome, JobRecord, JobStatus, JobStore, JobsError,
        QueueConfig,
    };

    const DEFAULT_SOURCE: &str = "registry:crates-io";
    const DEFAULT_INDEX_SOURCE: &str = "git:custom";
    const RATE_LIMIT_RETRY_AFTER_SECONDS: u64 = 60;
    const DEFAULT_TARBALL_SIZE_CAP_BYTES: u64 = 500_u64 * 1024 * 1024;
    const DEFAULT_GIT_SIZE_CAP_BYTES: u64 = 2_u64 * 1024 * 1024 * 1024;
    const DEFAULT_MAX_BUILD_SECONDS: u64 = 30 * 60;
    const DEFAULT_MAX_CONCURRENT_JOBS_PER_CALLER: u32 = 2;
    const DEFAULT_CALLS_PER_MINUTE: u32 = 10;
    const DEFAULT_MAX_QUEUED_JOBS_PER_OWNER: u32 = 20;
    const DEFAULT_QUEUE_SHARD_COUNT: u32 = 16;
    const ENQUEUE_CONFLICT_MAX_RETRIES: u32 = 10;
    const ENQUEUE_CONFLICT_BACKOFF_BASE_MS: u64 = 25;
    const ENQUEUE_CONFLICT_BACKOFF_MAX_MS: u64 = 500;
    const DESCRIBE_EXECUTION_TIMEOUT: Duration = Duration::from_secs(2);
    const STALE_JOB_REPAIR_AFTER: Duration = Duration::from_secs(60);

    pub const DEFAULT_INDEX_DRAINER_BATCH_LIMIT: usize = 8;
    pub const DEFAULT_INDEX_DRAINER_SCAN_LIMIT_PER_SHARD: usize = 32;
    pub const DEFAULT_INDEX_DRAINER_SCHEDULE_RATE_MINUTES: u64 = 1;
    pub const MAX_INDEX_GLOBAL_RUNNING_TOKENS: u32 = 32;

    #[derive(Debug, thiserror::Error)]
    pub enum McpHandlerError {
        #[error("invalid params: {0}")]
        InvalidParams(String),
        #[error("not found: {0}")]
        NotFound(String),
        #[error("internal: {0}")]
        Internal(String),
    }

    impl McpHandlerError {
        pub const fn json_rpc_code(&self) -> i32 {
            match self {
                Self::InvalidParams(_) => -32602,
                Self::NotFound(_) => -32004,
                Self::Internal(_) => -32603,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct IndexExecutionRequest {
        pub name: String,
        pub input: Value,
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
        fn describe_execution(
            &self,
            arn: &str,
        ) -> Result<Option<ExecutionOutcome>, McpHandlerError>;
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
    }

    impl ExecutionStatusChecker for SfnExecutionStatusChecker {
        fn describe_execution(
            &self,
            arn: &str,
        ) -> Result<Option<ExecutionOutcome>, McpHandlerError> {
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
                    .map_err(|error| {
                        McpHandlerError::Internal(format!("DescribeExecution: {error}"))
                    })
                })
            })
            .join()
            .map_err(|_| {
                McpHandlerError::Internal("DescribeExecution thread panicked".to_owned())
            })??;
            sfn_execution_outcome(output).map(Some)
        }
    }

    #[cfg(feature = "service")]
    impl crate::mcp::ExecutionStatusChecker for SfnExecutionStatusChecker {
        fn describe_execution(
            &self,
            arn: &str,
        ) -> Result<Option<crate::mcp::ExecutionOutcome>, crate::mcp::McpHandlerError> {
            let outcome =
                ExecutionStatusChecker::describe_execution(self, arn).map_err(|error| {
                    crate::mcp::McpHandlerError::Internal(format!(
                        "Code DescribeExecution reconciliation failed: {error}"
                    ))
                })?;
            Ok(outcome.map(|outcome| crate::mcp::ExecutionOutcome {
                status: match outcome.status {
                    ExecutionOutcomeStatus::Succeeded => {
                        crate::mcp::ExecutionOutcomeStatus::Succeeded
                    }
                    ExecutionOutcomeStatus::Failed => crate::mcp::ExecutionOutcomeStatus::Failed,
                    ExecutionOutcomeStatus::Running => crate::mcp::ExecutionOutcomeStatus::Running,
                },
                output: outcome.output,
                error: outcome.error,
            }))
        }
    }

    pub struct SfnIndexExecutionStarter {
        client: aws_sdk_sfn::Client,
        state_machine_arn: String,
    }

    impl SfnIndexExecutionStarter {
        pub fn new(client: aws_sdk_sfn::Client, state_machine_arn: String) -> Self {
            Self {
                client,
                state_machine_arn,
            }
        }
    }

    impl IndexExecutionStarter for SfnIndexExecutionStarter {
        fn start_execution<'a>(
            &'a self,
            request: IndexExecutionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<String, McpHandlerError>> + Send + 'a>> {
            Box::pin(async move {
                let input = serde_json::to_string(&request.input).map_err(|error| {
                    McpHandlerError::Internal(format!(
                        "external_index StartExecution input serialization failed: {error}"
                    ))
                })?;
                let output = self
                    .client
                    .start_execution()
                    .state_machine_arn(self.state_machine_arn.clone())
                    .name(request.name)
                    .input(input)
                    .send()
                    .await
                    .map_err(|error| {
                        McpHandlerError::Internal(format!(
                            "external_index StartExecution failed: {error}"
                        ))
                    })?;
                Ok(output.execution_arn().to_owned())
            })
        }
    }

    #[cfg(feature = "service")]
    impl crate::mcp::IndexExecutionStarter for SfnIndexExecutionStarter {
        fn start_execution<'a>(
            &'a self,
            request: crate::mcp::IndexExecutionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<String, crate::mcp::McpHandlerError>> + Send + 'a>>
        {
            Box::pin(async move {
                let input = serde_json::to_string(&request.input).map_err(|error| {
                    crate::mcp::McpHandlerError::Internal(format!(
                        "external_index StartExecution input serialization failed: {error}"
                    ))
                })?;
                let output = self
                    .client
                    .start_execution()
                    .state_machine_arn(self.state_machine_arn.clone())
                    .name(request.name)
                    .input(input)
                    .send()
                    .await
                    .map_err(|error| {
                        crate::mcp::McpHandlerError::Internal(format!(
                            "external_index StartExecution failed: {error}"
                        ))
                    })?;
                Ok(output.execution_arn().to_owned())
            })
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

    #[derive(Debug, Clone, Copy, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum ExternalIndexSourceKind {
        Git,
        Tarball,
    }

    #[derive(Debug, Clone)]
    struct ExternalIndexIdentity {
        source: String,
        package: String,
        revision: String,
        source_url: String,
        source_url_hash: String,
        source_kind: SourceKind,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExternalIndexStatusArgs {
        job_id: String,
    }

    pub async fn route_index(
        args: &Value,
        jobs: &dyn JobStore,
        backend: Option<&CodeBackend>,
        caller_id: &str,
    ) -> Result<Value, McpHandlerError> {
        route_index_with_dns(args, jobs, backend, caller_id, |parsed| {
            abuse::resolve_and_check_dns(parsed).map_err(|error| error.to_string())
        })
        .await
    }

    pub async fn route_index_with_dns<F>(
        args: &Value,
        jobs: &dyn JobStore,
        backend: Option<&CodeBackend>,
        caller_id: &str,
        dns_check: F,
    ) -> Result<Value, McpHandlerError>
    where
        F: FnOnce(&abuse::ParsedSourceUrl) -> Result<(), String>,
    {
        let args: ExternalIndexArgs = parse_args(args)?;
        validate_non_empty("package", &args.package)?;
        validate_non_empty("revision", &args.revision)?;
        validate_non_empty("source_url", &args.source_url)?;
        if let Some(source) = args.source.as_deref() {
            validate_non_empty("source", source)?;
        }

        let options = index_validate_options();
        let parsed_url = match abuse::validate(args.source_url.trim(), &options) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Ok(json!({
                    "status": "rejected",
                    "reason": format!("source_url: {error}")
                }))
            }
        };
        let identity = match canonicalize_index(&args, &parsed_url) {
            Ok(identity) => identity,
            Err(reason) => return Ok(json!({ "status": "rejected", "reason": reason })),
        };

        if !args.force.unwrap_or(false) {
            if let Some(backend) = backend {
                if let Some(value) = backend
                    .warm_index(&identity.source, &identity.package, &identity.revision)
                    .await
                    .map_err(|error| {
                        McpHandlerError::Internal(format!("external_index warm lookup: {error}"))
                    })?
                {
                    return Ok(value);
                }
            }
        }

        if let Err(error) = dns_check(&parsed_url) {
            return Ok(json!({
                "status": "rejected",
                "reason": format!("source_url: {error}")
            }));
        }
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
                }))
            }
            Err(error) => return Err(jobs_error("external_index rate limit failed", error)),
        }

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
        let owner = BacklogOwner::caller(caller_id);
        let mut attempt = 0;
        let result = loop {
            match jobs
                .enqueue_job(request.clone(), owner.clone(), &config)
                .await
            {
                Err(JobsError::Conflict) if attempt < ENQUEUE_CONFLICT_MAX_RETRIES => {
                    tokio::time::sleep(enqueue_conflict_backoff(attempt)).await;
                    attempt += 1;
                }
                result => break result,
            }
        };
        match result {
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
            Err(JobsError::Conflict) => Ok(json!({
                "status": "rejected",
                "reason": "conflict",
                "retry_after_seconds": 5
            })),
            Err(error) => Err(jobs_error("external_index enqueue failed", error)),
        }
    }

    pub async fn route_index_status_for_caller(
        args: &Value,
        jobs: &dyn JobStore,
        checker: Option<&dyn ExecutionStatusChecker>,
        caller_id: &str,
    ) -> Result<Value, McpHandlerError> {
        let args: ExternalIndexStatusArgs = parse_args(args)?;
        validate_non_empty("job_id", &args.job_id)?;
        let Some(record) = jobs
            .lookup_job(&args.job_id)
            .await
            .map_err(|error| jobs_error("external_index_status lookup failed", error))?
        else {
            return Ok(json!({ "status": "not_found" }));
        };
        if record.caller_id != caller_id {
            return Ok(json!({ "status": "not_found" }));
        }
        let record = update_stale_job(record, jobs, checker).await?;
        Ok(index_status_response(&record))
    }

    pub(crate) async fn update_stale_job(
        record: JobRecord,
        jobs: &dyn JobStore,
        checker: Option<&dyn ExecutionStatusChecker>,
    ) -> Result<JobRecord, McpHandlerError> {
        if record.status.is_terminal_for_quota() {
            jobs.release_running_quota(&record).await.map_err(|error| {
                jobs_error("external_index_status terminal quota release repair", error)
            })?;
            return Ok(record);
        }
        if !matches!(
            record.status,
            JobStatus::Queued | JobStatus::Dispatching | JobStatus::Running
        ) || !is_stale_job(&record)
        {
            return Ok(record);
        }
        let (Some(checker), Some(execution_arn)) = (checker, record.execution_arn.as_deref())
        else {
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
                    .and_then(|value| value.get("snapshot_id"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        McpHandlerError::Internal(
                            "DescribeExecution succeeded output missing snapshot_id".to_owned(),
                        )
                    })?;
                let row_counts = outcome
                    .output
                    .as_ref()
                    .and_then(|value| {
                        value
                            .get("rows_inserted")
                            .or_else(|| value.get("row_counts"))
                    })
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                jobs.mark_complete_and_release_running_quota(
                    &record.job_id,
                    snapshot_id,
                    row_counts,
                )
                .await
                .map_err(|error| jobs_error("external_index_status update complete failed", error))
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
                    .map_err(|error| {
                        jobs_error("external_index_status update failed status failed", error)
                    })
            }
        }
    }

    pub fn build_index_execution_request(record: &JobRecord) -> IndexExecutionRequest {
        let source_kind = match record.source_kind.trim() {
            "tarball" => SourceKind::Tarball,
            _ => SourceKind::Git,
        };
        let hostname = hostname_from_url(&record.source_url);
        let prefetch_source = match source_kind {
            SourceKind::Git => true,
            SourceKind::Tarball => !is_s3_https_hostname(&hostname),
        };
        let max_source_bytes = match source_kind {
            SourceKind::Git => env_u64("SPUR_CONTEXT_MAX_GIT_BYTES", DEFAULT_GIT_SIZE_CAP_BYTES),
            SourceKind::Tarball => env_u64(
                "SPUR_CONTEXT_MAX_TARBALL_BYTES",
                DEFAULT_TARBALL_SIZE_CAP_BYTES,
            ),
        };
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
                "max_source_bytes": max_source_bytes,
                "max_build_seconds": env_u64(
                    "SPUR_CONTEXT_MAX_BUILD_SECONDS",
                    DEFAULT_MAX_BUILD_SECONDS,
                )
            }
        });
        IndexExecutionRequest {
            name: record.job_id.clone(),
            input,
        }
    }

    pub fn index_queue_config() -> QueueConfig {
        QueueConfig {
            max_queued_per_owner: env_u32(
                "SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER",
                DEFAULT_MAX_QUEUED_JOBS_PER_OWNER,
            ),
            max_queued_global: env_u32("SPUR_INDEX_MAX_QUEUED_JOBS_GLOBAL", 0),
            max_running_per_owner: env_u32(
                "SPUR_INDEX_MAX_RUNNING_JOBS_PER_OWNER",
                env_u32(
                    "SPUR_INDEX_MAX_CONCURRENT_JOBS_PER_CALLER",
                    DEFAULT_MAX_CONCURRENT_JOBS_PER_CALLER,
                ),
            ),
            max_running_global: env_u32("SPUR_INDEX_MAX_RUNNING_JOBS_GLOBAL", 0)
                .min(MAX_INDEX_GLOBAL_RUNNING_TOKENS),
            shard_count: env_u32("SPUR_INDEX_QUEUE_SHARD_COUNT", DEFAULT_QUEUE_SHARD_COUNT),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct IndexDrainerLimits {
        pub max_dispatches_per_run: usize,
        pub scan_limit_per_shard: usize,
        pub rotation_interval_secs: u64,
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

    fn canonicalize_index(
        args: &ExternalIndexArgs,
        parsed_url: &abuse::ParsedSourceUrl,
    ) -> Result<ExternalIndexIdentity, &'static str> {
        let package = args.package.trim();
        let revision = args.revision.trim();
        let source_url = args.source_url.trim();
        let source = args.source.as_deref().map(str::trim);
        let (source, package, source_url, source_kind) = if let Some((url_package, url_revision)) =
            crates_io_download_coordinates(source_url, &parsed_url.hostname)
        {
            let package = normalize_crates_io_package(package);
            if package != normalize_crates_io_package(url_package) {
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
            (
                source,
                package,
                format!(
                    "https://crates.io/api/v1/crates/{}/{revision}/download",
                    url_package.to_ascii_lowercase()
                ),
                SourceKind::Tarball,
            )
        } else {
            let source_kind = match args.source_kind {
                Some(ExternalIndexSourceKind::Git) => SourceKind::Git,
                Some(ExternalIndexSourceKind::Tarball) => SourceKind::Tarball,
                None => parsed_url.source_kind,
            };
            (
                source.unwrap_or(DEFAULT_INDEX_SOURCE).to_owned(),
                package.to_owned(),
                source_url.to_owned(),
                source_kind,
            )
        };
        Ok(ExternalIndexIdentity {
            source,
            package,
            revision: revision.to_owned(),
            source_url_hash: source_url_hash(&source_url),
            source_url,
            source_kind,
        })
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
            Err(McpHandlerError::InvalidParams(format!(
                "field '{field}' must be non-empty"
            )))
        } else {
            Ok(())
        }
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

    fn is_stale_job(record: &JobRecord) -> bool {
        let Some(updated_millis) = record.updated_at.parse::<u128>().ok() else {
            return false;
        };
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis().saturating_sub(updated_millis))
            .is_some_and(|elapsed| elapsed >= STALE_JOB_REPAIR_AFTER.as_millis())
    }

    fn sfn_execution_outcome(
        output: aws_sdk_sfn::operation::describe_execution::DescribeExecutionOutput,
    ) -> Result<ExecutionOutcome, McpHandlerError> {
        let status = match output.status() {
            AwsExecutionStatus::Succeeded => ExecutionOutcomeStatus::Succeeded,
            AwsExecutionStatus::Running | AwsExecutionStatus::PendingRedrive => {
                ExecutionOutcomeStatus::Running
            }
            AwsExecutionStatus::Failed
            | AwsExecutionStatus::Aborted
            | AwsExecutionStatus::TimedOut => ExecutionOutcomeStatus::Failed,
            other => match other.as_str() {
                "SUCCEEDED" => ExecutionOutcomeStatus::Succeeded,
                "RUNNING" | "PENDING_REDRIVE" => ExecutionOutcomeStatus::Running,
                _ => ExecutionOutcomeStatus::Failed,
            },
        };
        let value = output
            .output()
            .map(|raw| {
                serde_json::from_str(raw).map_err(|error| {
                    McpHandlerError::Internal(format!("DescribeExecution output JSON: {error}"))
                })
            })
            .transpose()?;
        let error = match (output.error(), output.cause()) {
            (Some(error), Some(cause)) if !cause.trim().is_empty() => {
                Some(format!("{error}: {cause}"))
            }
            (Some(error), _) => Some(error.to_owned()),
            (None, Some(cause)) if !cause.trim().is_empty() => Some(format!("execution: {cause}")),
            (None, _) if status == ExecutionOutcomeStatus::Failed => {
                Some(format!("execution: {}", output.status().as_str()))
            }
            _ => None,
        };
        Ok(ExecutionOutcome {
            status,
            output: value,
            error,
        })
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
            let code = record.error_code.as_deref().unwrap_or("execution");
            let detail = record.error_detail.as_deref().unwrap_or("failed");
            response["error"] = json!({
                "code": code,
                "detail": detail,
                "retriable": matches!(code, "fetch" | "commit" | "spot_interrupted" | "timeout")
            });
        }
        response
    }

    fn jobs_error(context: &str, error: JobsError) -> McpHandlerError {
        match error {
            JobsError::NotFound => McpHandlerError::NotFound(format!("{context}: {error}")),
            error => McpHandlerError::Internal(format!("{context}: {error}")),
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

    fn normalize_crates_io_package(package: &str) -> String {
        package.trim().to_ascii_lowercase().replace('_', "-")
    }

    fn is_crates_io_source_alias(source: &str) -> bool {
        ["crates.io", "crates-io", DEFAULT_SOURCE]
            .iter()
            .any(|alias| source.eq_ignore_ascii_case(alias))
    }

    fn hostname_from_url(url: &str) -> String {
        let after_scheme = url.split("://").nth(1).unwrap_or(url);
        after_scheme
            .split('/')
            .next()
            .unwrap_or(after_scheme)
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase()
    }

    fn is_s3_https_hostname(hostname: &str) -> bool {
        let Some(prefix) = hostname.strip_suffix(".amazonaws.com") else {
            return false;
        };
        prefix == "s3"
            || prefix
                .strip_prefix("s3.")
                .is_some_and(|region| !region.is_empty() && !region.contains('.'))
            || prefix
                .strip_suffix(".s3")
                .is_some_and(|bucket| !bucket.is_empty())
            || prefix.rsplit_once(".s3.").is_some_and(|(bucket, region)| {
                !bucket.is_empty() && !region.is_empty() && !region.contains('.')
            })
    }

    fn split_job_error(error: &str) -> (&str, &str) {
        error.split_once(':').map_or_else(
            || (error.trim(), ""),
            |(code, detail)| (code.trim(), detail.trim()),
        )
    }

    fn index_rate_limit_per_minute() -> u32 {
        env_u32("SPUR_INDEX_RATE_LIMIT_PER_MINUTE", DEFAULT_CALLS_PER_MINUTE)
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
}

#[cfg(feature = "code-lambda")]
mod code {
    use std::collections::BTreeMap;
    use std::env;
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use aws_config::BehaviorVersion;
    use lambda_runtime::{Error, LambdaEvent};
    use secrecy::ExposeSecret;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use tokio::sync::{Mutex, OnceCell};

    use super::code_control::{self, McpHandlerError};
    use super::{tool_is_eligible, BackendKind};
    use crate::api_keys::{
        generate_api_key, ApiKeyRecord, ApiKeyScopes, ApiKeyStore, ApiKeyStoreError,
        CreateKeyRecord, DynamoDbApiKeyStore, KeyEnvironment, RevokeResult,
    };
    use crate::artifact_cache::{ArtifactCache, S3ArtifactFetcher};
    use crate::auth::{self, AuthConfig, AuthFailure, RequestRoute};
    use crate::code_backend::{
        CatalogRequest, CodeBackend, CodeBackendError, CodeEdgesRequest, CodeReadRequest,
        CodeSearchRequest,
    };
    use crate::drainer;
    use crate::jobs::DynamoDbJobStore;
    use crate::lambda_http::{
        authenticated_caller_id, classify_route, json_response, reject_api_key_auth_on_wrong_route,
        reject_jwt_auth_on_wrong_route, ApiGatewayRequest, ApiGatewayResponse,
    };
    use crate::serving_registry::ServingRegistry;

    const DEFAULT_SOURCE: &str = "registry:crates-io";
    const CODE_CACHE_CAPACITY_ENV: &str = "SPUR_CONTEXT_CODE_CACHE_BYTES";

    static CODE_BACKEND: OnceCell<Mutex<Option<ActiveCodeBackend>>> = OnceCell::const_new();
    static CODE_S3_CLIENT: OnceCell<aws_sdk_s3::Client> = OnceCell::const_new();
    static CODE_AWS_CLIENTS: OnceCell<CodeAwsClients> = OnceCell::const_new();

    struct CodeAwsClients {
        dynamodb: aws_sdk_dynamodb::Client,
        sfn: aws_sdk_sfn::Client,
    }

    #[derive(Clone)]
    struct ActiveCodeBackend {
        identity: RegistryIdentity,
        backend: Arc<CodeBackend>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RegistryIdentity {
        generation: i64,
        uri: String,
        sha256: String,
        bytes: u64,
    }

    #[derive(Debug, Deserialize)]
    struct ToolRequest {
        tool: String,
        #[serde(default)]
        args: Value,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CodeToolErrorKind {
        InvalidParams,
        NotFound,
        Internal,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{message}")]
    struct CodeToolError {
        kind: CodeToolErrorKind,
        message: String,
    }

    impl CodeToolError {
        fn invalid(message: impl Into<String>) -> Self {
            Self {
                kind: CodeToolErrorKind::InvalidParams,
                message: format!("invalid params: {}", message.into()),
            }
        }

        fn not_found(message: impl Into<String>) -> Self {
            Self {
                kind: CodeToolErrorKind::NotFound,
                message: format!("not found: {}", message.into()),
            }
        }

        fn retryable(code: &'static str) -> Self {
            Self {
                kind: CodeToolErrorKind::Internal,
                message: format!("code backend temporarily unavailable: {code} (retryable=true)"),
            }
        }

        fn internal(message: impl Into<String>) -> Self {
            Self {
                kind: CodeToolErrorKind::Internal,
                message: message.into(),
            }
        }

        const fn json_rpc_code(&self) -> i32 {
            match self.kind {
                CodeToolErrorKind::InvalidParams => -32602,
                CodeToolErrorKind::NotFound => -32004,
                CodeToolErrorKind::Internal => -32603,
            }
        }
    }

    pub async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
        #[cfg(test)]
        if event
            .payload
            .get("__spur_count_handler")
            .and_then(Value::as_bool)
            == Some(true)
        {
            super::CODE_HANDLER_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if is_scheduled_drainer_event(&event.payload) {
            let summary = drain_queued_jobs().await?;
            return Ok(json!({
                "operation": "drain_queued_jobs",
                "dispatched": summary.dispatched,
                "skipped": summary.skipped,
                "failed": summary.failed,
                "repaired": summary.repaired
            }));
        }
        let request =
            serde_json::from_value::<ApiGatewayRequest>(event.payload).map_err(|error| {
                lambda_error(format!(
                    "failed to deserialize API Gateway invocation: {error}"
                ))
            })?;
        let response = handle_api_gateway_request(request).await?;
        serde_json::to_value(response).map_err(Error::from)
    }

    async fn handle_api_gateway_request(
        api_gateway_request: ApiGatewayRequest,
    ) -> Result<ApiGatewayResponse, Error> {
        if let Some(response) = handle_reserved_route_before_body(&api_gateway_request) {
            return response;
        }
        let route = classify_route(&api_gateway_request);
        if !super::control_route_is_eligible(BackendKind::Code, route) {
            return reserved_route_disabled_response(route);
        }
        if let Err(error) = reject_api_key_auth_on_wrong_route(&api_gateway_request) {
            return authorization_error_response(error);
        }
        if matches!(
            route,
            RequestRoute::ApiKeyCreate | RequestRoute::ApiKeyList | RequestRoute::ApiKeyRevoke
        ) {
            let Some(config) = ApiKeyManagementConfig::from_environment() else {
                return reserved_route_disabled_response(route);
            };
            let store = api_key_store().await;
            return handle_api_key_management(
                route,
                &api_gateway_request,
                &store,
                &config,
                unix_now_seconds()?,
            )
            .await;
        }
        if let Err(error) = reject_jwt_auth_on_wrong_route(&api_gateway_request) {
            return authorization_error_response(error);
        }

        let request = match parse_tool_request(&api_gateway_request) {
            Ok(request) => request,
            Err(error) => return code_error_response(error),
        };
        if !tool_is_eligible(BackendKind::Code, &request.tool) {
            return code_error_response(CodeToolError::invalid(format!(
                "tool `{}` is not available on the Code Lambda",
                request.tool
            )));
        }

        let authenticated_caller =
            match authorize_request(&api_gateway_request, route, &request.tool) {
                Ok(caller) => caller,
                Err(error) => return authorization_error_response(error),
            };

        let result = match request.tool.as_str() {
            "external_index" => {
                let Some(caller_id) = authenticated_caller.as_deref() else {
                    return authorization_error_response(AuthFailure::MissingContext);
                };
                let jobs = job_store().await;
                let backend = code_backend().await.ok();
                let result =
                    code_control::route_index(&request.args, &jobs, backend.as_deref(), caller_id)
                        .await;
                if result
                    .as_ref()
                    .is_ok_and(|value| value["status"] == "queued")
                {
                    kick_drainer().await;
                }
                result.map_err(code_control_error)
            }
            "external_index_status" => {
                let Some(caller_id) = authenticated_caller.as_deref() else {
                    return authorization_error_response(AuthFailure::MissingContext);
                };
                let jobs = job_store().await;
                let checker = status_checker().await;
                code_control::route_index_status_for_caller(
                    &request.args,
                    &jobs,
                    Some(&checker),
                    caller_id,
                )
                .await
                .map_err(code_control_error)
            }
            _ => match code_backend().await {
                Ok(backend) => handle_tool(&request.tool, &request.args, &backend).await,
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(value) => json_response(200, &value),
            Err(error) => code_error_response(error),
        }
    }

    fn authorize_request(
        request: &ApiGatewayRequest,
        route: RequestRoute,
        tool: &str,
    ) -> Result<Option<String>, AuthFailure> {
        match route {
            RequestRoute::ApiKeyMcp => {
                let context = api_key_context(request)?;
                auth::authorize_api_key_tool(tool, Some(&context))
                    .map(|decision| Some(decision.identity.caller_id().to_owned()))
            }
            RequestRoute::OAuth => {
                let config = match AuthConfig::from_environment()? {
                    Some(config) => config,
                    None => return Err(AuthFailure::AuthDisabled),
                };
                let claims = request
                    .request_context
                    .as_ref()
                    .and_then(|context| context.authorizer.as_ref())
                    .and_then(|authorizer| authorizer.jwt.as_ref())
                    .and_then(|jwt| jwt.claims.as_ref());
                auth::authorize_oauth_tool_now(&config, tool, claims)
                    .map(|decision| Some(decision.identity.caller_id().to_owned()))
            }
            RequestRoute::Legacy if matches!(tool, "external_index" | "external_index_status") => {
                authenticated_caller_id(request, anonymous_mutations_allowed())
                    .map(Some)
                    .map_err(|_| AuthFailure::MissingContext)
            }
            RequestRoute::Legacy => Ok(None),
            _ => Err(AuthFailure::WrongRoute),
        }
    }

    fn api_key_context(
        request: &ApiGatewayRequest,
    ) -> Result<crate::api_key_authorizer::ApiKeyAuthContext, AuthFailure> {
        let value = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.lambda.as_ref())
            .ok_or(AuthFailure::MissingContext)?;
        crate::api_key_authorizer::ApiKeyAuthContext::from_value(value)
            .map_err(|_| AuthFailure::InvalidApiKeyContext)
    }

    fn anonymous_mutations_allowed() -> bool {
        matches!(
            env::var("SPUR_CONTEXT_ALLOW_ANONYMOUS_MUTATIONS")
                .ok()
                .as_deref()
                .map(str::trim),
            Some("1") | Some("true") | Some("TRUE") | Some("yes")
        )
    }

    const SECONDS_PER_DAY: u64 = 86_400;
    const DEFAULT_API_KEY_TTL_DAYS: u64 = 90;
    const MAX_API_KEY_TTL_DAYS: u64 = 365;
    const API_KEY_LIST_LIMIT: usize = 100;
    const API_KEY_CREATE_MAX_ATTEMPTS: usize = 3;
    const API_KEY_CURSOR_MAX_LEN: usize = 128;
    const LOGIN_REDIRECT_MAX_RAW_QUERY_LEN: usize = 8_192;
    const HUMAN_CALLBACK_URL: &str = "http://127.0.0.1:8765/callback";
    const LOGIN_REDIRECT_ENABLED_ENV: &str = "SPUR_CONTEXT_LOGIN_REDIRECT_ENABLED";

    #[derive(Debug, Clone)]
    struct ApiKeyManagementConfig {
        auth: AuthConfig,
        environment: KeyEnvironment,
        default_ttl_days: u64,
        max_ttl_days: u64,
    }

    impl ApiKeyManagementConfig {
        fn new(
            auth: AuthConfig,
            environment: KeyEnvironment,
            default_ttl_days: u64,
            max_ttl_days: u64,
        ) -> Option<Self> {
            (default_ttl_days > 0
                && default_ttl_days <= max_ttl_days
                && max_ttl_days <= MAX_API_KEY_TTL_DAYS)
                .then_some(Self {
                    auth,
                    environment,
                    default_ttl_days,
                    max_ttl_days,
                })
        }

        fn from_environment() -> Option<Self> {
            if !api_key_routes_configured() {
                return None;
            }
            let auth = AuthConfig::from_environment().ok().flatten()?;
            let environment = match env::var("SPUR_API_KEY_ENVIRONMENT")
                .unwrap_or_else(|_| "live".to_owned())
                .as_str()
            {
                "live" => KeyEnvironment::Live,
                "test" => KeyEnvironment::Test,
                _ => return None,
            };
            let default_ttl_days =
                environment_u64("SPUR_API_KEY_DEFAULT_TTL_DAYS", DEFAULT_API_KEY_TTL_DAYS)?;
            let max_ttl_days = environment_u64("SPUR_API_KEY_MAX_TTL_DAYS", MAX_API_KEY_TTL_DAYS)?;
            Self::new(auth, environment, default_ttl_days, max_ttl_days)
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CreateApiKeyRequest {
        name: String,
        scopes: Vec<String>,
        #[serde(default)]
        expires_at: Option<u64>,
    }

    async fn handle_api_key_management(
        route: RequestRoute,
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        config: &ApiKeyManagementConfig,
        now: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        let claims = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.jwt.as_ref())
            .and_then(|jwt| jwt.claims.as_ref());
        let decision = match auth::authorize_key_management(&config.auth, claims, now) {
            Ok(decision) => decision,
            Err(error) => return authorization_error_response(error),
        };
        let owner_id = decision.identity.caller_id();
        match route {
            RequestRoute::ApiKeyCreate => {
                create_api_key(request, store, config, owner_id, now).await
            }
            RequestRoute::ApiKeyList => list_api_keys(request, store, owner_id).await,
            RequestRoute::ApiKeyRevoke => revoke_api_key(request, store, owner_id, now).await,
            _ => reserved_route_disabled_response(route),
        }
    }

    async fn create_api_key(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        config: &ApiKeyManagementConfig,
        owner_id: &str,
        now: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        if request.is_base64_encoded {
            return management_error_response(400, "invalid_request");
        }
        let Some(create) = request
            .body
            .as_deref()
            .and_then(|body| serde_json::from_str::<CreateApiKeyRequest>(body).ok())
        else {
            return management_error_response(400, "invalid_request");
        };
        let scope_refs = create.scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let scopes = match ApiKeyScopes::parse(&scope_refs) {
            Ok(scopes) => scopes,
            Err(_) => return management_error_response(400, "invalid_scope"),
        };
        let default_expiry = now.checked_add(
            config
                .default_ttl_days
                .checked_mul(SECONDS_PER_DAY)
                .ok_or_else(|| lambda_error("API key default expiry overflow"))?,
        );
        let max_expiry = now.checked_add(
            config
                .max_ttl_days
                .checked_mul(SECONDS_PER_DAY)
                .ok_or_else(|| lambda_error("API key maximum expiry overflow"))?,
        );
        let (Some(default_expiry), Some(max_expiry)) = (default_expiry, max_expiry) else {
            return management_error_response(503, "key_service_unavailable");
        };
        let expires_at = create.expires_at.unwrap_or(default_expiry);
        if expires_at <= now || expires_at > max_expiry {
            return management_error_response(400, "invalid_expiry");
        }
        for attempt in 0..API_KEY_CREATE_MAX_ATTEMPTS {
            let generated = match generate_api_key(
                config.environment,
                owner_id,
                &create.name,
                scopes.clone(),
                now,
                expires_at,
            ) {
                Ok(generated) => generated,
                Err(crate::api_keys::ApiKeyError::InvalidName) => {
                    return management_error_response(400, "invalid_name")
                }
                Err(crate::api_keys::ApiKeyError::InvalidScope) => {
                    return management_error_response(400, "invalid_scope")
                }
                Err(crate::api_keys::ApiKeyError::InvalidExpiry) => {
                    return management_error_response(400, "invalid_expiry")
                }
                Err(_) => return management_error_response(503, "key_service_unavailable"),
            };
            match store
                .create_key(CreateKeyRecord::new(generated.record.clone()))
                .await
            {
                Ok(()) => {
                    let plaintext = generated.plaintext.expose_secret().to_owned();
                    let record = generated.record;
                    return one_time_secret_response(
                        201,
                        &json!({
                            "key": plaintext,
                            "key_id": record.public_id,
                            "name": record.name,
                            "scopes": record.scopes.as_strings(),
                            "created_at": record.created_at,
                            "expires_at": record.expires_at,
                        }),
                    );
                }
                Err(ApiKeyStoreError::DuplicatePublicId)
                    if attempt + 1 < API_KEY_CREATE_MAX_ATTEMPTS => {}
                Err(error) => return api_key_store_error_response(error),
            }
        }
        management_error_response(503, "key_store_unavailable")
    }

    async fn list_api_keys(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        owner_id: &str,
    ) -> Result<ApiGatewayResponse, Error> {
        let query = request.query_string_parameters.as_ref();
        let cursor = query
            .and_then(|parameters| parameters.get("cursor"))
            .map(String::as_str);
        if cursor.is_some_and(|value| value.is_empty() || value.len() > API_KEY_CURSOR_MAX_LEN) {
            return management_error_response(400, "invalid_request");
        }
        let limit = match query.and_then(|parameters| parameters.get("limit")) {
            Some(value) => match parse_api_key_list_limit(value) {
                Some(limit) => limit,
                None => return management_error_response(400, "invalid_request"),
            },
            None => API_KEY_LIST_LIMIT,
        };
        let page = match store.list_owner_keys(owner_id, cursor, limit).await {
            Ok(page) => page,
            Err(error) => return api_key_store_error_response(error),
        };
        let keys = page.keys.iter().map(api_key_metadata).collect::<Vec<_>>();
        json_response(
            200,
            &json!({ "keys": keys, "next_cursor": page.next_cursor }),
        )
    }

    async fn revoke_api_key(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        owner_id: &str,
        now: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        let key_id = request
            .raw_path
            .as_deref()
            .or(request.path.as_deref())
            .and_then(|path| path.strip_prefix("/auth/api-keys/"))
            .filter(|key_id| valid_public_key_id(key_id));
        let Some(key_id) = key_id else {
            return management_error_response(404, "not_found");
        };
        match store.revoke_key(owner_id, key_id, now).await {
            Ok(RevokeResult::Revoked | RevokeResult::AlreadyRevoked) => {
                json_response(200, &json!({ "key_id": key_id, "status": "revoked" }))
            }
            Ok(RevokeResult::NotFound) => management_error_response(404, "not_found"),
            Err(error) => api_key_store_error_response(error),
        }
    }

    fn parse_api_key_list_limit(value: &str) -> Option<usize> {
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return None;
        }
        value
            .parse()
            .ok()
            .filter(|limit| (1..=API_KEY_LIST_LIMIT).contains(limit))
    }

    fn api_key_metadata(record: &ApiKeyRecord) -> Value {
        json!({
            "key_id": record.public_id,
            "name": record.name,
            "scopes": record.scopes.as_strings(),
            "status": record.status.as_str(),
            "created_at": record.created_at,
            "expires_at": record.expires_at,
            "revoked_at": record.revoked_at,
        })
    }

    fn api_key_store_error_response(error: ApiKeyStoreError) -> Result<ApiGatewayResponse, Error> {
        match error {
            ApiKeyStoreError::OwnerLimit => management_error_response(409, "key_limit_reached"),
            ApiKeyStoreError::InvalidRequest => management_error_response(400, "invalid_request"),
            _ => management_error_response(503, "key_store_unavailable"),
        }
    }

    fn management_error_response(
        status_code: u16,
        code: &'static str,
    ) -> Result<ApiGatewayResponse, Error> {
        json_response(status_code, &json!({ "error": { "code": code } }))
    }

    fn valid_public_key_id(key_id: &str) -> bool {
        key_id.len() == 26
            && key_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
    }

    #[derive(Debug, Clone)]
    struct DiscoveryConfig {
        issuer: String,
        human_client_id: String,
        authorization_endpoint: String,
        token_endpoint: String,
        service_base_url: String,
    }

    impl DiscoveryConfig {
        fn from_environment() -> Option<Self> {
            if !environment_is_truthy("SPUR_COGNITO_AUTH_ENABLED") {
                return None;
            }
            Self::new(
                env::var("SPUR_COGNITO_ISSUER").ok()?,
                env::var("SPUR_COGNITO_HUMAN_CLIENT_ID").ok()?,
                env::var("SPUR_COGNITO_AUTHORIZATION_ENDPOINT").ok()?,
                env::var("SPUR_COGNITO_TOKEN_ENDPOINT").ok()?,
                env::var("SPUR_CONTEXT_SERVICE_BASE_URL").ok()?,
            )
        }

        fn new(
            issuer: String,
            human_client_id: String,
            authorization_endpoint: String,
            token_endpoint: String,
            service_base_url: String,
        ) -> Option<Self> {
            let bounded = [
                issuer.as_str(),
                human_client_id.as_str(),
                authorization_endpoint.as_str(),
                token_endpoint.as_str(),
                service_base_url.as_str(),
            ]
            .into_iter()
            .all(|value| {
                !value.is_empty()
                    && value.len() <= 2_048
                    && value.trim() == value
                    && !value.chars().any(char::is_control)
            });
            let secure_urls = [
                &issuer,
                &authorization_endpoint,
                &token_endpoint,
                &service_base_url,
            ]
            .into_iter()
            .all(|value| value.starts_with("https://"));
            let service_origin_is_bounded =
                https_origin(&service_base_url).is_some() && !service_base_url.contains(['?', '#']);
            let authorization_origin =
                exact_https_endpoint_origin(&authorization_endpoint, "/oauth2/authorize");
            let token_origin = exact_https_endpoint_origin(&token_endpoint, "/oauth2/token");
            if !bounded
                || !secure_urls
                || !service_origin_is_bounded
                || authorization_origin.is_none()
                || authorization_origin != token_origin
            {
                return None;
            }
            Some(Self {
                issuer,
                human_client_id,
                authorization_endpoint,
                token_endpoint,
                service_base_url: service_base_url.trim_end_matches('/').to_owned(),
            })
        }
    }

    #[derive(Debug, Serialize)]
    struct DiscoveryDocument {
        schema_version: u8,
        issuer: String,
        human_client_id: String,
        human_callback_url: String,
        authorization_endpoint: String,
        token_endpoint: String,
        supported_scopes: Vec<String>,
        api_key_auth_enabled: bool,
        api_key_mcp_url: String,
        api_key_management_url: String,
    }

    fn discovery_document(config: &DiscoveryConfig) -> Option<DiscoveryDocument> {
        let resource_server_id = env::var("SPUR_COGNITO_RESOURCE_SERVER_ID")
            .unwrap_or_else(|_| "urn:spur:context-service".to_owned());
        if resource_server_id.trim().is_empty()
            || resource_server_id.len() > 256
            || resource_server_id.chars().any(char::is_control)
        {
            return None;
        }
        Some(DiscoveryDocument {
            schema_version: 1,
            issuer: config.issuer.clone(),
            human_client_id: config.human_client_id.clone(),
            human_callback_url: HUMAN_CALLBACK_URL.to_owned(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint: config.token_endpoint.clone(),
            supported_scopes: [
                "external.index",
                "external.read",
                "external.status",
                "keys.manage",
            ]
            .into_iter()
            .map(|scope| format!("{resource_server_id}/{scope}"))
            .collect(),
            api_key_auth_enabled: ApiKeyManagementConfig::from_environment().is_some(),
            api_key_mcp_url: format!("{}{}", config.service_base_url, auth::API_KEY_MCP_PATH),
            api_key_management_url: format!(
                "{}{}",
                config.service_base_url,
                auth::API_KEY_MANAGEMENT_PATH
            ),
        })
    }

    fn handle_reserved_route_before_body(
        request: &ApiGatewayRequest,
    ) -> Option<Result<ApiGatewayResponse, Error>> {
        let route = classify_route(request);
        match route {
            RequestRoute::ReservedUnavailable => Some(reserved_route_disabled_response(route)),
            RequestRoute::Discovery => {
                let Some(config) = DiscoveryConfig::from_environment() else {
                    return Some(reserved_route_disabled_response(route));
                };
                Some(match discovery_document(&config) {
                    Some(document) => serde_json::to_value(document)
                        .map_err(Error::from)
                        .and_then(|value| json_response(200, &value)),
                    None => reserved_route_disabled_response(route),
                })
            }
            RequestRoute::Login => {
                let Some(config) = login_redirect_config_from_environment() else {
                    return Some(reserved_route_disabled_response(route));
                };
                Some(
                    login_redirect_response(&config, request.raw_query_string.as_deref())
                        .map_or_else(|| reserved_route_disabled_response(route), Ok),
                )
            }
            route if route.is_api_key() && !api_key_routes_configured() => {
                Some(reserved_route_disabled_response(route))
            }
            _ => None,
        }
    }

    fn login_redirect_config_from_environment() -> Option<DiscoveryConfig> {
        environment_is_truthy(LOGIN_REDIRECT_ENABLED_ENV)
            .then(DiscoveryConfig::from_environment)
            .flatten()
    }

    fn login_redirect_response(
        config: &DiscoveryConfig,
        raw_query_string: Option<&str>,
    ) -> Option<ApiGatewayResponse> {
        let query = raw_query_string.unwrap_or_default();
        if !safe_raw_oauth_query(query) {
            return None;
        }
        let mut location = config.authorization_endpoint.clone();
        if !query.is_empty() {
            location.push('?');
            location.push_str(query);
        }
        Some(ApiGatewayResponse {
            status_code: 302,
            headers: BTreeMap::from([
                ("cache-control".to_owned(), "no-store".to_owned()),
                ("content-length".to_owned(), "0".to_owned()),
                ("location".to_owned(), location),
                ("pragma".to_owned(), "no-cache".to_owned()),
                ("referrer-policy".to_owned(), "no-referrer".to_owned()),
                ("x-content-type-options".to_owned(), "nosniff".to_owned()),
            ]),
            body: String::new(),
            is_base64_encoded: false,
        })
    }

    fn safe_raw_oauth_query(query: &str) -> bool {
        if query.len() > LOGIN_REDIRECT_MAX_RAW_QUERY_LEN {
            return false;
        }
        let bytes = query.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'%' {
                let (Some(&high), Some(&low)) = (bytes.get(index + 1), bytes.get(index + 2)) else {
                    return false;
                };
                let (Some(high), Some(low)) = (hex_value(high), hex_value(low)) else {
                    return false;
                };
                if ((high << 4) | low).is_ascii_control() {
                    return false;
                }
                index += 3;
                continue;
            }
            if !(byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                        | b'/'
                        | b'?'
                ))
            {
                return false;
            }
            index += 1;
        }
        true
    }

    const fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    fn https_origin(value: &str) -> Option<&str> {
        let authority_and_path = value.strip_prefix("https://")?;
        let authority_len = authority_and_path
            .find(['/', '?', '#'])
            .unwrap_or(authority_and_path.len());
        let authority = &authority_and_path[..authority_len];
        let valid = !authority.is_empty()
            && authority.len() <= 253
            && authority.split('.').all(|label| {
                let bytes = label.as_bytes();
                !bytes.is_empty()
                    && bytes.len() <= 63
                    && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                    && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
                    && bytes
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            });
        valid.then_some(&value[.."https://".len() + authority_len])
    }

    fn exact_https_endpoint_origin<'a>(value: &'a str, path: &str) -> Option<&'a str> {
        let origin = https_origin(value)?;
        (value.strip_prefix(origin) == Some(path)).then_some(origin)
    }

    fn reserved_route_disabled_response(_route: RequestRoute) -> Result<ApiGatewayResponse, Error> {
        json_response(404, &json!({ "error": { "code": "route_unavailable" } }))
    }

    fn api_key_routes_configured() -> bool {
        environment_is_truthy("SPUR_API_KEY_AUTH_ENABLED")
            && env::var("SPUR_CONTEXT_API_KEYS_TABLE")
                .ok()
                .is_some_and(|name| !name.trim().is_empty() && name.len() <= 255)
            && DiscoveryConfig::from_environment().is_some()
    }

    fn environment_is_truthy(name: &str) -> bool {
        matches!(
            env::var(name).ok().as_deref().map(str::trim),
            Some("1") | Some("true") | Some("TRUE") | Some("yes")
        )
    }

    fn environment_u64(name: &str, default: u64) -> Option<u64> {
        match env::var(name) {
            Ok(value) => value.parse().ok(),
            Err(env::VarError::NotPresent) => Some(default),
            Err(env::VarError::NotUnicode(_)) => None,
        }
    }

    fn unix_now_seconds() -> Result<u64, Error> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| lambda_error("system clock is before Unix epoch"))
    }

    fn parse_tool_request(request: &ApiGatewayRequest) -> Result<ToolRequest, CodeToolError> {
        if request.is_base64_encoded {
            return Err(CodeToolError::invalid(
                "base64-encoded API Gateway bodies are not supported",
            ));
        }
        let body = request
            .body
            .as_deref()
            .ok_or_else(|| CodeToolError::invalid("missing request body"))?;
        if let Some(tool) = routed_tool_name(request) {
            let value: Value = serde_json::from_str(body).map_err(|error| {
                CodeToolError::invalid(format!("failed to parse request JSON body: {error}"))
            })?;
            let args = value.get("args").cloned().unwrap_or(value);
            return Ok(ToolRequest {
                tool: tool.to_owned(),
                args,
            });
        }
        serde_json::from_str(body).map_err(|error| {
            CodeToolError::invalid(format!("failed to parse request JSON body: {error}"))
        })
    }

    fn routed_tool_name(request: &ApiGatewayRequest) -> Option<&'static str> {
        let path = request.raw_path.as_deref().or(request.path.as_deref())?;
        match path.trim_end_matches('/').rsplit('/').next() {
            Some("index") => Some("external_index"),
            Some("index_status") => Some("external_index_status"),
            _ => None,
        }
    }

    async fn code_backend() -> Result<Arc<CodeBackend>, CodeToolError> {
        let pointer_uri = env::var("SPUR_CATALOG_S3_URI")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CodeToolError::retryable("serving_registry_unavailable"))?;
        let client = code_s3_client().await;
        let pointer_bytes = download_s3_object(client, &pointer_uri).await?;
        let pointer: CodeServingPointer = serde_json::from_slice(&pointer_bytes)
            .map_err(|_| CodeToolError::retryable("serving_registry_unavailable"))?;
        let identity = pointer.registry_identity()?;
        activate_code_backend(
            CODE_BACKEND
                .get_or_init(|| async { Mutex::new(None) })
                .await,
            identity,
            || load_code_backend(client, pointer),
        )
        .await
    }

    async fn code_s3_client() -> &'static aws_sdk_s3::Client {
        CODE_S3_CLIENT
            .get_or_init(|| async {
                let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
                aws_sdk_s3::Client::new(&config)
            })
            .await
    }

    async fn activate_code_backend<F, Fut>(
        slot: &Mutex<Option<ActiveCodeBackend>>,
        identity: RegistryIdentity,
        loader: F,
    ) -> Result<Arc<CodeBackend>, CodeToolError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CodeBackend, CodeToolError>>,
    {
        let mut active = slot.lock().await;
        if let Some(current) = active.as_ref() {
            if current.identity == identity {
                return Ok(Arc::clone(&current.backend));
            }
            if current.identity.generation > identity.generation {
                return Err(CodeToolError::retryable("serving_registry_unavailable"));
            }
        }

        let backend = Arc::new(loader().await?);
        if backend.generation() != identity.generation {
            return Err(CodeToolError::retryable("serving_registry_unavailable"));
        }
        *active = Some(ActiveCodeBackend {
            identity,
            backend: Arc::clone(&backend),
        });
        Ok(backend)
    }

    async fn load_code_backend(
        s3: &aws_sdk_s3::Client,
        pointer: CodeServingPointer,
    ) -> Result<CodeBackend, CodeToolError> {
        let capacity = env::var(CODE_CACHE_CAPACITY_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| CodeToolError::retryable("artifact_cache_unavailable"))?;

        let registry_ref = pointer.registry_identity()?;
        let registry_bytes = download_s3_object(s3, &registry_ref.uri).await?;
        verify_object(&registry_bytes, &registry_ref)?;
        let registry = ServingRegistry::from_json_slice(&registry_bytes)
            .map_err(|_| CodeToolError::retryable("serving_registry_unavailable"))?;
        if registry.generation != pointer.generation {
            return Err(CodeToolError::retryable("serving_registry_unavailable"));
        }

        let cache = ArtifactCache::new(
            PathBuf::from("/tmp/spur-context-code"),
            capacity,
            Arc::new(S3ArtifactFetcher::new(s3.clone())),
        )
        .map_err(|_| CodeToolError::retryable("artifact_cache_unavailable"))?;
        CodeBackend::new(registry, cache).map_err(code_backend_error)
    }

    #[derive(Debug, Deserialize)]
    struct CodeServingPointer {
        generation: i64,
        status: String,
        serving_registry_uri: Option<String>,
        serving_registry_sha256: Option<String>,
        serving_registry_bytes: Option<u64>,
    }

    impl CodeServingPointer {
        fn registry_identity(&self) -> Result<RegistryIdentity, CodeToolError> {
            if self.generation <= 0 || self.status != "published" {
                return Err(CodeToolError::retryable("serving_registry_unavailable"));
            }
            let reference = RegistryIdentity {
                generation: self.generation,
                uri: self
                    .serving_registry_uri
                    .clone()
                    .ok_or_else(|| CodeToolError::retryable("serving_registry_unavailable"))?,
                sha256: self
                    .serving_registry_sha256
                    .clone()
                    .ok_or_else(|| CodeToolError::retryable("serving_registry_unavailable"))?,
                bytes: self
                    .serving_registry_bytes
                    .ok_or_else(|| CodeToolError::retryable("serving_registry_unavailable"))?,
            };
            if reference.bytes == 0
                || reference.sha256.len() != 64
                || !reference
                    .sha256
                    .as_bytes()
                    .iter()
                    .all(u8::is_ascii_hexdigit)
                || parse_s3_uri(&reference.uri).is_none()
            {
                return Err(CodeToolError::retryable("serving_registry_unavailable"));
            }
            Ok(reference)
        }
    }

    async fn download_s3_object(
        client: &aws_sdk_s3::Client,
        uri: &str,
    ) -> Result<Vec<u8>, CodeToolError> {
        let (bucket, key) = parse_s3_uri(uri)
            .ok_or_else(|| CodeToolError::retryable("serving_registry_unavailable"))?;
        let output = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|_| CodeToolError::retryable("serving_registry_unavailable"))?;
        let bytes = output
            .body
            .collect()
            .await
            .map_err(|_| CodeToolError::retryable("serving_registry_unavailable"))?;
        Ok(bytes.into_bytes().to_vec())
    }

    fn parse_s3_uri(uri: &str) -> Option<(&str, &str)> {
        let value = uri.strip_prefix("s3://")?;
        let (bucket, key) = value.split_once('/')?;
        (!bucket.is_empty() && !key.is_empty()).then_some((bucket, key))
    }

    fn verify_object(bytes: &[u8], reference: &RegistryIdentity) -> Result<(), CodeToolError> {
        if bytes.len() as u64 != reference.bytes {
            return Err(CodeToolError::retryable("serving_registry_unavailable"));
        }
        let actual = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if actual != reference.sha256 {
            return Err(CodeToolError::retryable("serving_registry_unavailable"));
        }
        Ok(())
    }

    async fn handle_tool(
        name: &str,
        args: &Value,
        backend: &CodeBackend,
    ) -> Result<Value, CodeToolError> {
        match name {
            "external_catalog" => {
                let args: CatalogArgs = parse_args(args)?;
                args.validate()?;
                let revision_or_ref = args.revision_ref().map(str::to_owned);
                backend
                    .catalog(CatalogRequest {
                        source: args.source().to_owned(),
                        package: args.package,
                        revision_or_ref,
                        path: args.path,
                        name_filter: args.name_filter,
                        limit: args.limit.unwrap_or(50).clamp(1, 200),
                        cursor: args.cursor,
                    })
                    .await
                    .map_err(code_backend_error)
            }
            "external_code_search" => {
                let args: CodeSearchArgs = parse_args(args)?;
                args.validate()?;
                let revision_or_ref = args.revision_ref().map(str::to_owned);
                backend
                    .search(CodeSearchRequest {
                        source: args.source().to_owned(),
                        package: args.package,
                        revision_or_ref,
                        query: args.query,
                        symbol_kind: args.symbol_kind,
                        limit: args.limit.unwrap_or(20).clamp(1, 200),
                    })
                    .await
                    .map_err(code_backend_error)
            }
            "external_code_read" => {
                let args: CodeReadArgs = parse_args(args)?;
                validate_non_empty("selector", &args.selector)?;
                backend
                    .read(CodeReadRequest {
                        source: args.source().to_owned(),
                        selector: args.selector,
                        context_lines: args.context_lines.unwrap_or(0),
                    })
                    .await
                    .map_err(code_backend_tool_error)
            }
            "external_code_callers" | "external_code_callees" => {
                let args: CodeEdgesArgs = parse_args(args)?;
                validate_non_empty("selector", &args.selector)?;
                let request = CodeEdgesRequest {
                    source: args.source().to_owned(),
                    selector: args.selector,
                    include_unresolved: args.include_unresolved.unwrap_or(false),
                };
                if name == "external_code_callers" {
                    backend.callers(request).await
                } else {
                    backend.callees(request).await
                }
                .map_err(code_backend_tool_error)
            }
            _ => Err(CodeToolError::invalid(format!(
                "unknown context-service MCP tool: {name}"
            ))),
        }
    }

    fn code_backend_error(error: CodeBackendError) -> CodeToolError {
        match error {
            CodeBackendError::PackageUnavailable => {
                CodeToolError::not_found("package revision not found")
            }
            CodeBackendError::InvalidCursor => CodeToolError::invalid("invalid catalog cursor"),
            CodeBackendError::InvalidSelector(message) => CodeToolError::invalid(message),
            CodeBackendError::AmbiguousSelector { .. } => {
                CodeToolError::invalid("external selector is ambiguous")
            }
            CodeBackendError::SymbolNotFound(selector) => {
                CodeToolError::not_found(format!("symbol not found: {selector}"))
            }
            error => CodeToolError::retryable(error.code()),
        }
    }

    fn code_backend_tool_error(error: CodeBackendError) -> CodeToolError {
        code_backend_error(error)
    }

    fn parse_args<T>(args: &Value) -> Result<T, CodeToolError>
    where
        T: for<'de> Deserialize<'de>,
    {
        serde_json::from_value(args.clone()).map_err(|error| {
            CodeToolError::invalid(format!("failed to parse tool arguments: {error}"))
        })
    }

    fn validate_non_empty(field: &str, value: &str) -> Result<(), CodeToolError> {
        if value.trim().is_empty() {
            Err(CodeToolError::invalid(format!(
                "field '{field}' must be non-empty"
            )))
        } else {
            Ok(())
        }
    }

    fn validate_revision_choice(
        revision: Option<&str>,
        ref_name: Option<&str>,
    ) -> Result<(), CodeToolError> {
        if revision.is_some() && ref_name.is_some() {
            Err(CodeToolError::invalid(
                "fields 'revision' and 'ref' are mutually exclusive",
            ))
        } else {
            Ok(())
        }
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
        fn validate(&self) -> Result<(), CodeToolError> {
            for (name, value) in [
                ("source", self.source.as_deref()),
                ("package", self.package.as_deref()),
                ("revision", self.revision.as_deref()),
                ("ref", self.ref_name.as_deref()),
            ] {
                if let Some(value) = value {
                    validate_non_empty(name, value)?;
                }
            }
            validate_revision_choice(self.revision.as_deref(), self.ref_name.as_deref())?;
            if self.package.is_none()
                && (self.revision.is_some() || self.ref_name.is_some() || self.path.is_some())
            {
                return Err(CodeToolError::invalid(
                    "field 'package' is required when using revision, ref, or path",
                ));
            }
            if self.package.is_some() && self.revision_ref().is_none() && self.path.is_some() {
                return Err(CodeToolError::invalid(
                    "field 'path' requires revision or ref",
                ));
            }
            Ok(())
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
        fn validate(&self) -> Result<(), CodeToolError> {
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
    struct CodeEdgesArgs {
        selector: String,
        source: Option<String>,
        include_unresolved: Option<bool>,
    }

    impl CodeEdgesArgs {
        fn source(&self) -> &str {
            self.source.as_deref().unwrap_or(DEFAULT_SOURCE)
        }
    }

    fn code_error_response(error: CodeToolError) -> Result<ApiGatewayResponse, Error> {
        let status = if error.kind == CodeToolErrorKind::Internal {
            500
        } else {
            200
        };
        json_response(
            status,
            &json!({
                "error": {
                    "code": error.json_rpc_code(),
                    "message": error.to_string()
                }
            }),
        )
    }

    fn code_control_error(error: McpHandlerError) -> CodeToolError {
        match error {
            McpHandlerError::InvalidParams(message) => CodeToolError::invalid(message),
            McpHandlerError::NotFound(message) => CodeToolError::not_found(message),
            McpHandlerError::Internal(message) => CodeToolError::internal(message),
        }
    }

    fn is_scheduled_drainer_event(payload: &Value) -> bool {
        payload.get("source").and_then(Value::as_str) == Some("aws.events")
            && payload.get("detail-type").and_then(Value::as_str) == Some("Scheduled Event")
            && payload.pointer("/detail/operation").and_then(Value::as_str)
                == Some("drain_queued_jobs")
    }

    async fn code_aws_clients() -> &'static CodeAwsClients {
        CODE_AWS_CLIENTS
            .get_or_init(|| async {
                let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
                CodeAwsClients {
                    dynamodb: aws_sdk_dynamodb::Client::new(&config),
                    sfn: aws_sdk_sfn::Client::new(&config),
                }
            })
            .await
    }

    async fn job_store() -> DynamoDbJobStore {
        DynamoDbJobStore::new(code_aws_clients().await.dynamodb.clone())
    }

    async fn api_key_store() -> DynamoDbApiKeyStore {
        DynamoDbApiKeyStore::new(code_aws_clients().await.dynamodb.clone())
    }

    async fn status_checker() -> code_control::SfnExecutionStatusChecker {
        code_control::SfnExecutionStatusChecker::new(code_aws_clients().await.sfn.clone())
    }

    async fn sfn_starter() -> Result<code_control::SfnIndexExecutionStarter, Error> {
        let state_machine_arn = env::var("SPUR_INDEX_STATE_MACHINE_ARN").map_err(|error| {
            lambda_error(format!(
                "SPUR_INDEX_STATE_MACHINE_ARN environment variable is required: {error}"
            ))
        })?;
        Ok(code_control::SfnIndexExecutionStarter::new(
            code_aws_clients().await.sfn.clone(),
            state_machine_arn,
        ))
    }

    async fn drain_queued_jobs() -> Result<drainer::DrainSummary, Error> {
        let jobs = job_store().await;
        let starter = sfn_starter().await?;
        let checker = status_checker().await;
        let config = code_control::index_queue_config();
        let limits = code_control::index_drainer_limits();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        Ok(drainer::Drainer::new(&jobs, &starter, config)
            .with_limits(limits.max_dispatches_per_run, limits.scan_limit_per_shard)
            .with_rotation_interval_secs(limits.rotation_interval_secs)
            .with_checker(&checker)
            .drain(now)
            .await)
    }

    async fn kick_drainer() {
        if let Err(error) = drain_queued_jobs().await {
            eprintln!("[lambda] best-effort Code drainer kick failed: {error}");
        }
    }

    fn authorization_error_response(error: AuthFailure) -> Result<ApiGatewayResponse, Error> {
        let code = if error.status_code() == 401 {
            "authentication_failed"
        } else {
            "authorization_failed"
        };
        json_response(
            error.status_code(),
            &json!({
                "error": {
                    "code": code,
                    "reason": error.reason(),
                }
            }),
        )
    }

    fn one_time_secret_response(
        status_code: u16,
        value: &Value,
    ) -> Result<ApiGatewayResponse, Error> {
        let mut response = json_response(status_code, value)?;
        response
            .headers
            .insert("cache-control".to_owned(), "no-store".to_owned());
        response
            .headers
            .insert("pragma".to_owned(), "no-cache".to_owned());
        Ok(response)
    }

    fn lambda_error(message: impl Into<String>) -> Error {
        std::io::Error::other(message.into()).into()
    }

    #[cfg(test)]
    mod correction_tests {
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex as StdMutex};

        use async_trait::async_trait;
        use serde_json::json;

        use super::*;
        use crate::artifact_cache::{ArtifactFetchError, ArtifactFetcher, ArtifactStream};
        use crate::jobs::{
            BacklogOwner, CreateJobOutcome, CreateJobRequest, EnqueueOutcome, JobRecord, JobStatus,
            JobStore, JobsError, QueueConfig,
        };
        use crate::serving_registry::SERVING_REGISTRY_SCHEMA_VERSION;

        fn api_key_control_request(scopes: &str) -> ApiGatewayRequest {
            serde_json::from_value(json!({
                "rawPath": "/mcp/api-key/code",
                "requestContext": {
                    "authorizer": { "lambda": {
                        "auth_context_version": 1,
                        "auth_kind": "api_key",
                        "owner_id": "cognito:user:fixture-human",
                        "key_id": "aaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "scopes": scopes
                    } },
                    "http": { "method": "POST" }
                }
            }))
            .expect("API-key control request")
        }

        #[test]
        fn code_control_authorization_preserves_api_key_owner() {
            let request = api_key_control_request("external.index external.status");
            for tool in ["external_index", "external_index_status"] {
                let caller = authorize_request(&request, classify_route(&request), tool)
                    .expect("scoped API key should authorize control tool");
                assert_eq!(
                    serde_json::to_value(caller).unwrap(),
                    json!("cognito:user:fixture-human"),
                    "{tool} must retain the authorized owner for job ownership"
                );
            }
        }

        #[test]
        fn code_control_authorization_rejects_missing_api_key_scope_and_context() {
            let request = api_key_control_request("external.read");
            for tool in ["external_index", "external_index_status"] {
                assert_eq!(
                    authorize_request(&request, classify_route(&request), tool),
                    Err(AuthFailure::MissingScope)
                );
            }
            let mut fallback = request;
            fallback
                .request_context
                .as_mut()
                .unwrap()
                .authorizer
                .as_mut()
                .unwrap()
                .lambda = None;
            fallback
                .request_context
                .as_mut()
                .unwrap()
                .authorizer
                .as_mut()
                .unwrap()
                .principal_id = Some("untrusted-fallback".to_owned());
            assert_eq!(
                authorize_request(&fallback, classify_route(&fallback), "external_index"),
                Err(AuthFailure::MissingContext)
            );
        }

        #[test]
        fn code_control_authorization_preserves_iam_owner() {
            let request: ApiGatewayRequest = serde_json::from_value(json!({
                "rawPath": "/mcp/code",
                "requestContext": {
                    "authorizer": { "iam": {
                        "accountId": "123456789012",
                        "userId": "AROAFIXTURE:session",
                        "userArn": "arn:aws:sts::123456789012:assumed-role/fixture/session"
                    } },
                    "http": { "method": "POST" }
                }
            }))
            .unwrap();
            for tool in ["external_index", "external_index_status"] {
                let caller = authorize_request(&request, classify_route(&request), tool).unwrap();
                assert_eq!(
                    serde_json::to_value(caller).unwrap(),
                    json!(authenticated_caller_id(&request, false).unwrap())
                );
            }
        }

        struct EmptyFetcher;

        #[async_trait]
        impl ArtifactFetcher for EmptyFetcher {
            async fn fetch(&self, _uri: &str) -> Result<ArtifactStream, ArtifactFetchError> {
                Ok(Box::pin(tokio::io::empty()))
            }
        }

        struct ControlStore {
            record: StdMutex<JobRecord>,
        }

        impl ControlStore {
            fn new(status: JobStatus) -> Self {
                Self {
                    record: StdMutex::new(JobRecord {
                        job_id: "job-123".to_owned(),
                        status,
                        source: "git:custom".to_owned(),
                        package: "demo".to_owned(),
                        revision: "1.0.0".to_owned(),
                        source_url: "https://example.com/demo.tar.gz".to_owned(),
                        source_url_hash: "fnv1a64:test".to_owned(),
                        source_kind: "tarball".to_owned(),
                        caller_id: "caller-1".to_owned(),
                        execution_arn: Some("arn:execution:job-123".to_owned()),
                        attempt: 1,
                        stage: Some("build".to_owned()),
                        snapshot_id: None,
                        row_counts: None,
                        error_code: None,
                        error_detail: None,
                        created_at: "0".to_owned(),
                        updated_at: u128::MAX.to_string(),
                        owner_kind: None,
                        owner_id: None,
                        queue_shard: None,
                        queue_sort_key: None,
                        next_eligible_at: None,
                        dispatched_at: None,
                    }),
                }
            }

            fn record(&self) -> JobRecord {
                self.record.lock().expect("control store lock").clone()
            }
        }

        #[async_trait]
        impl JobStore for ControlStore {
            async fn create_or_get_active_job(
                &self,
                _request: CreateJobRequest,
            ) -> Result<CreateJobOutcome, JobsError> {
                Ok(CreateJobOutcome::Existing(self.record()))
            }

            async fn record_execution_started(
                &self,
                _job_id: &str,
                execution_arn: &str,
            ) -> Result<JobRecord, JobsError> {
                let mut record = self.record.lock().expect("control store lock");
                record.status = JobStatus::Running;
                record.execution_arn = Some(execution_arn.to_owned());
                Ok(record.clone())
            }

            async fn update_stage(
                &self,
                _job_id: &str,
                status: JobStatus,
                stage: &str,
            ) -> Result<JobRecord, JobsError> {
                let mut record = self.record.lock().expect("control store lock");
                record.status = status;
                record.stage = Some(stage.to_owned());
                Ok(record.clone())
            }

            async fn mark_complete(
                &self,
                _job_id: &str,
                snapshot_id: i64,
                row_counts: Value,
            ) -> Result<JobRecord, JobsError> {
                let mut record = self.record.lock().expect("control store lock");
                record.status = JobStatus::Complete;
                record.snapshot_id = Some(snapshot_id);
                record.row_counts = Some(row_counts);
                Ok(record.clone())
            }

            async fn mark_failed(
                &self,
                _job_id: &str,
                code: &str,
                detail: &str,
            ) -> Result<JobRecord, JobsError> {
                let mut record = self.record.lock().expect("control store lock");
                record.status = JobStatus::Failed;
                record.error_code = Some(code.to_owned());
                record.error_detail = Some(detail.to_owned());
                Ok(record.clone())
            }

            async fn lookup_job(&self, job_id: &str) -> Result<Option<JobRecord>, JobsError> {
                Ok((job_id == "job-123").then(|| self.record()))
            }

            async fn release_dedupe_if_owner(&self, _record: &JobRecord) -> Result<(), JobsError> {
                Ok(())
            }

            async fn enqueue_job(
                &self,
                _request: CreateJobRequest,
                _owner: BacklogOwner,
                _config: &QueueConfig,
            ) -> Result<EnqueueOutcome, JobsError> {
                let mut record = self.record.lock().expect("control store lock");
                record.status = JobStatus::Queued;
                Ok(EnqueueOutcome::Enqueued(record.clone()))
            }
        }

        fn backend(generation: i64) -> CodeBackend {
            let cache = ArtifactCache::new(
                PathBuf::from(format!("/tmp/spur-code-lambda-red-{generation}")),
                1,
                Arc::new(EmptyFetcher),
            )
            .expect("test cache should initialize");
            CodeBackend::new(
                ServingRegistry {
                    schema_version: SERVING_REGISTRY_SCHEMA_VERSION,
                    generation,
                    packages: Vec::new(),
                },
                cache,
            )
            .expect("test backend should initialize")
        }

        fn request(path: &str, method: &str, body: Option<Value>) -> ApiGatewayRequest {
            serde_json::from_value(json!({
                "rawPath": path,
                "requestContext": { "http": { "method": method } },
                "body": body.map(|value| serde_json::to_string(&value).unwrap()),
                "isBase64Encoded": false
            }))
            .expect("request should deserialize")
        }

        fn management_request(method: &str, path: &str, body: Option<Value>) -> ApiGatewayRequest {
            serde_json::from_value(json!({
                "rawPath": path,
                "body": body.map(|value| value.to_string()),
                "requestContext": {
                    "authorizer": {
                        "jwt": {
                            "claims": {
                                "iss": "https://issuer.example/pool",
                                "token_use": "access",
                                "client_id": "human-client",
                                "sub": "code-owner",
                                "exp": "2000000000",
                                "scope": "urn:spur:context-service/keys.manage"
                            }
                        }
                    },
                    "http": { "method": method }
                }
            }))
            .expect("management request should deserialize")
        }

        fn management_config() -> ApiKeyManagementConfig {
            ApiKeyManagementConfig::new(
                AuthConfig::new(
                    "https://issuer.example/pool",
                    "human-client",
                    ["m2m-client"],
                    std::iter::empty::<&str>(),
                    "urn:spur:context-service",
                ),
                KeyEnvironment::Live,
                90,
                365,
            )
            .expect("management config should be valid")
        }

        #[tokio::test]
        async fn code_preserves_discovery_login_and_api_key_route_semantics() {
            for (path, method, allowed_statuses) in [
                (
                    "/.well-known/oauth-protected-resource",
                    "GET",
                    &[200, 404][..],
                ),
                ("/auth/login", "GET", &[302, 404][..]),
                ("/auth/api-keys", "POST", &[401, 403, 404][..]),
                ("/auth/api-keys", "GET", &[401, 403, 404][..]),
                ("/auth/api-keys/key-123", "DELETE", &[401, 403, 404][..]),
            ] {
                let response = handle_api_gateway_request(request(path, method, None))
                    .await
                    .expect("control route should return a bounded response");
                assert!(
                    allowed_statuses.contains(&response.status_code),
                    "Code must execute the configured or fail-closed control semantics for {method} {path}, got {}",
                    response.status_code,
                );
            }
        }

        #[tokio::test]
        async fn code_api_key_management_create_list_and_revoke_uses_the_real_seam() {
            let now = 1_700_000_000;
            let store = crate::api_keys::FakeApiKeyStore::new();
            let config = management_config();
            let created = handle_api_key_management(
                RequestRoute::ApiKeyCreate,
                &management_request(
                    "POST",
                    "/auth/api-keys",
                    Some(json!({
                        "name": "code-client",
                        "scopes": ["external.read", "external.index", "external.status"]
                    })),
                ),
                &store,
                &config,
                now,
            )
            .await
            .expect("Code key creation should return a response");
            assert_eq!(created.status_code, 201);
            assert_eq!(
                created.headers.get("cache-control").map(String::as_str),
                Some("no-store")
            );
            let created_body: Value =
                serde_json::from_str(&created.body).expect("created body should be JSON");
            let key_id = created_body["key_id"]
                .as_str()
                .expect("created key id should be present")
                .to_owned();
            assert!(created_body["key"]
                .as_str()
                .is_some_and(|key| !key.is_empty()));

            let listed = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &management_request("GET", "/auth/api-keys", None),
                &store,
                &config,
                now,
            )
            .await
            .expect("Code key listing should return a response");
            assert_eq!(listed.status_code, 200);
            let listed_body: Value =
                serde_json::from_str(&listed.body).expect("listed body should be JSON");
            assert_eq!(listed_body["keys"][0]["key_id"], key_id);
            assert!(listed_body["keys"][0].get("key").is_none());

            let revoked = handle_api_key_management(
                RequestRoute::ApiKeyRevoke,
                &management_request("DELETE", &format!("/auth/api-keys/{key_id}"), None),
                &store,
                &config,
                now,
            )
            .await
            .expect("Code key revocation should return a response");
            assert_eq!(revoked.status_code, 200);
            let revoked_body: Value =
                serde_json::from_str(&revoked.body).expect("revoked body should be JSON");
            assert_eq!(revoked_body["status"], "revoked");
        }

        #[tokio::test]
        async fn cold_and_forced_index_use_the_control_plane_instead_of_registry_failure() {
            let backend = backend(1);
            for force in [false, true] {
                let store = ControlStore::new(JobStatus::Queued);
                let result = code_control::route_index_with_dns(
                    &json!({
                        "package": "demo",
                        "revision": "1.0.0",
                        "source_url": "https://example.com/demo.tar.gz",
                        "source_kind": "tarball",
                        "force": force
                    }),
                    &store,
                    Some(&backend),
                    "caller-1",
                    |_| Ok(()),
                )
                .await
                .expect("cold/forced index should reach the injected control seam");
                assert_eq!(result["status"], "queued");
                assert_eq!(result["job_id"], "job-123");
            }
        }

        #[tokio::test]
        async fn index_status_uses_job_store_reconciliation() {
            let store = ControlStore::new(JobStatus::Running);
            let result = code_control::route_index_status_for_caller(
                &json!({ "job_id": "job-123" }),
                &store,
                None,
                "caller-1",
            )
            .await
            .expect("status should return a bounded response");
            assert_eq!(result["status"], "running");
            assert_eq!(result["job_id"], "job-123");
        }

        #[tokio::test]
        async fn live_pointer_generation_advancement_replaces_the_backend() {
            let slot = Mutex::new(None);
            let first_identity = RegistryIdentity {
                generation: 1,
                uri: "s3://registry/g1.json".to_owned(),
                sha256: "1".repeat(64),
                bytes: 1,
            };
            let first =
                activate_code_backend(&slot, first_identity.clone(), || async { Ok(backend(1)) })
                    .await
                    .expect("g should initialize");
            assert_eq!(first.generation(), 1);

            let reused = activate_code_backend(&slot, first_identity, || async {
                panic!("strongly identical pointer must reuse the active backend")
            })
            .await
            .expect("strong identity should reuse g");
            assert!(Arc::ptr_eq(&first, &reused));

            let advanced = activate_code_backend(
                &slot,
                RegistryIdentity {
                    generation: 2,
                    uri: "s3://registry/g2.json".to_owned(),
                    sha256: "2".repeat(64),
                    bytes: 2,
                },
                || async { Ok(backend(2)) },
            )
            .await
            .expect("g+1 should activate");
            assert_eq!(advanced.generation(), 2, "g+1 must replace g");
        }
    }
}

#[cfg(feature = "knowledge-lambda")]
pub async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
    handler_for(BackendKind::Knowledge, event).await
}

#[cfg(feature = "knowledge-lambda")]
pub use knowledge::route_index_status_control_plane;

#[cfg(feature = "knowledge-lambda")]
mod knowledge {

    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use lambda_runtime::{Error, LambdaEvent};
    use secrecy::ExposeSecret;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use super::{tool_is_eligible, BackendKind};
    use crate::api_keys::{
        generate_api_key, ApiKeyRecord, ApiKeyScopes, ApiKeyStore, ApiKeyStoreError,
        CreateKeyRecord, DynamoDbApiKeyStore, KeyEnvironment, RevokeResult,
    };
    use crate::auth::{self, AuthConfig, AuthDecision, AuthFailure, RequestRoute};
    use crate::catalog::{self, CatalogResolver};
    use crate::drainer;
    use crate::jobs::{DynamoDbJobStore, JobStore};
    use crate::lambda_http::{
        authenticated_caller_id, classify_route, json_response, reject_api_key_auth_on_wrong_route,
        reject_jwt_auth_on_wrong_route, ApiGatewayRequest, ApiGatewayResponse,
    };
    #[cfg(test)]
    use crate::lambda_http::{caller_id, ApiGatewayHttp, ApiGatewayRequestContext};
    use crate::mcp::{self, McpHandlerError};

    pub static CATALOG_RESOLVER: OnceLock<Mutex<Option<CatalogCacheEntry>>> = OnceLock::new();
    static AWS_CLIENTS: OnceLock<AwsClients> = OnceLock::new();
    static SNAPSHOT_CACHE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    pub struct CatalogCacheEntry {
        catalog_dsn: String,
        catalog_etag: Option<String>,
        resolver: CatalogResolver,
    }

    struct PreparedCatalog {
        cache_key: String,
        catalog_etag: Option<String>,
        source: PreparedCatalogSource,
    }

    enum PreparedCatalogSource {
        FrozenSnapshot {
            local_path: PathBuf,
            data_path: String,
        },
        Direct {
            catalog_dsn: String,
        },
    }

    #[derive(Debug, Deserialize)]
    struct ToolRequest {
        tool: String,
        #[serde(default)]
        args: Value,
    }

    #[cfg(test)]
    tokio::task_local! {
        static TEST_DRAINER_SUMMARY: drainer::DrainSummary;
    }

    #[cfg(test)]
    async fn with_test_drainer_summary<T>(
        summary: drainer::DrainSummary,
        future: impl Future<Output = T>,
    ) -> T {
        TEST_DRAINER_SUMMARY.scope(summary, future).await
    }

    pub async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
        #[cfg(test)]
        if event
            .payload
            .get("__spur_count_handler")
            .and_then(Value::as_bool)
            == Some(true)
        {
            super::KNOWLEDGE_HANDLER_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        #[cfg(test)]
        if let Ok(summary) = TEST_DRAINER_SUMMARY.try_with(Clone::clone) {
            return handle_event_with_drainer(event, async move { Ok(summary) }).await;
        }
        handle_event_with_drainer(event, drain_queued_jobs()).await
    }

    async fn handle_event_with_drainer<F>(
        event: LambdaEvent<Value>,
        drain_future: F,
    ) -> Result<Value, Error>
    where
        F: Future<Output = Result<drainer::DrainSummary, Error>>,
    {
        if is_scheduled_drainer_event(&event.payload) {
            let summary = drain_future.await?;
            return Ok(json!({
                "operation": "drain_queued_jobs",
                "dispatched": summary.dispatched,
                "skipped": summary.skipped,
                "failed": summary.failed,
                "repaired": summary.repaired
            }));
        }

        let request =
            serde_json::from_value::<ApiGatewayRequest>(event.payload).map_err(|error| {
                lambda_error(format!(
                    "failed to deserialize API Gateway invocation: {error}"
                ))
            })?;
        let response = handle_api_gateway_request(request).await?;
        serde_json::to_value(response).map_err(Error::from)
    }

    fn is_scheduled_drainer_event(payload: &Value) -> bool {
        payload.get("source").and_then(Value::as_str) == Some("aws.events")
            && payload.get("detail-type").and_then(Value::as_str) == Some("Scheduled Event")
            && payload.pointer("/detail/operation").and_then(Value::as_str)
                == Some("drain_queued_jobs")
    }

    async fn handle_api_gateway_request(
        api_gateway_request: ApiGatewayRequest,
    ) -> Result<ApiGatewayResponse, Error> {
        let route = classify_route(&api_gateway_request);
        if !super::control_route_is_eligible(BackendKind::Knowledge, route) {
            return reserved_route_disabled_response(route);
        }
        if let Err(error) = reject_api_key_auth_on_wrong_route(&api_gateway_request) {
            return authorization_error_response(error);
        }
        if let Err(error) = reject_jwt_auth_on_wrong_route(&api_gateway_request) {
            return authorization_error_response(error);
        }

        let api_key_context = if route == RequestRoute::ApiKeyMcp {
            match api_key_context(&api_gateway_request) {
                Ok(context) => Some(context),
                Err(error) => return authorization_error_response(error),
            }
        } else {
            None
        };

        let request = parse_tool_request(&api_gateway_request);
        let request = match request {
            Ok(request) => request,
            Err(error) => return tool_error_response(error),
        };

        if !tool_is_eligible(BackendKind::Knowledge, &request.tool) {
            return tool_error_response(McpHandlerError::InvalidParams(format!(
                "tool `{}` is not available on the Knowledge Lambda",
                request.tool
            )));
        }

        let authenticated_caller = if route == RequestRoute::ApiKeyMcp {
            match auth::authorize_api_key_tool(&request.tool, api_key_context.as_ref()) {
                Ok(decision) => Some(decision.identity.caller_id().to_owned()),
                Err(error) => return authorization_error_response(error),
            }
        } else if route == RequestRoute::OAuth {
            let config = match AuthConfig::from_environment() {
                Ok(Some(config)) => config,
                Ok(None) => return authorization_error_response(AuthFailure::AuthDisabled),
                Err(error) => return authorization_error_response(error),
            };
            match authorize_oauth_request_now(&api_gateway_request, &request.tool, &config) {
                Ok(decision) => Some(decision.identity.caller_id().to_owned()),
                Err(error) => return authorization_error_response(error),
            }
        } else {
            match request.tool.as_str() {
                "external_index" | "external_index_status" => Some(
                    match authenticated_caller_id(
                        &api_gateway_request,
                        anonymous_mutations_allowed(),
                    ) {
                        Ok(caller_id) => caller_id,
                        Err(error) => {
                            return auth_error_response(McpHandlerError::InvalidParams(
                                error.to_string(),
                            ))
                        }
                    },
                ),
                _ => None,
            }
        };

        let result = match request.tool.as_str() {
            "external_index_status" => {
                let jobs = job_store();
                let checker = status_checker();
                let caller_id = authenticated_caller
                    .as_deref()
                    .expect("external_index_status authenticated caller should be available");
                route_index_status_control_plane(&request.args, &jobs, &checker, caller_id).await
            }
            "external_index" => {
                let prepared_catalog = prepare_catalog().await?;
                let jobs = job_store();
                let sfn_client = sfn_client()?;
                let caller_id = authenticated_caller
                    .as_deref()
                    .expect("external_index authenticated caller should be available");
                let warm_result = if let Some(prepared_catalog) = &prepared_catalog {
                    with_initialized_catalog(prepared_catalog, |catalog| {
                        mcp::route_index_warm_lookup(&request.args, catalog)
                    })?
                } else {
                    Ok(None)
                };
                let result = match warm_result {
                    Ok(Some(response)) => Ok(response),
                    Ok(None) => {
                        mcp::route_index_without_catalog(
                            &request.args,
                            &jobs,
                            &sfn_client,
                            caller_id,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                // Best-effort drainer kick: if the job was accepted into the queue,
                // try to dispatch queued work immediately for lower latency. The
                // scheduled EventBridge drainer remains the correctness fallback;
                // failure to kick must not affect the admission response.
                if let Ok(value) = &result {
                    if is_queued_job_response(value) {
                        kick_drainer().await;
                    }
                }
                result
            }
            _ => {
                let prepared_catalog = prepare_catalog().await?;
                let Some(prepared_catalog) = prepared_catalog else {
                    return match mcp::handle_tool_without_catalog(&request.tool, &request.args) {
                        Ok(value) => json_response(200, &value),
                        Err(error) => tool_error_response(error),
                    };
                };
                with_initialized_catalog(&prepared_catalog, |catalog| {
                    let db = catalog.connection();
                    mcp::handle_tool_sync(&request.tool, &request.args, db, catalog)
                })?
            }
        };

        match result {
            Ok(value) => json_response(200, &value),
            Err(McpHandlerError::Internal(message)) => json_response(
                500,
                &json!({
                    "error": {
                        "code": McpHandlerError::Internal(message.clone()).json_rpc_code(),
                        "message": message
                    }
                }),
            ),
            Err(error) => tool_error_response(error),
        }
    }

    const SECONDS_PER_DAY: u64 = 86_400;
    const DEFAULT_API_KEY_TTL_DAYS: u64 = 90;
    const MAX_API_KEY_TTL_DAYS: u64 = 365;
    const API_KEY_LIST_LIMIT: usize = 100;
    const API_KEY_CREATE_MAX_ATTEMPTS: usize = 3;
    const HUMAN_CALLBACK_URL: &str = "http://127.0.0.1:8765/callback";
    const API_KEY_CURSOR_MAX_LEN: usize = 128;
    const LOGIN_REDIRECT_MAX_RAW_QUERY_LEN: usize = 8_192;
    const LOGIN_REDIRECT_ENABLED_ENV: &str = "SPUR_CONTEXT_LOGIN_REDIRECT_ENABLED";

    #[derive(Debug, Clone)]
    struct ApiKeyManagementConfig {
        auth: AuthConfig,
        environment: KeyEnvironment,
        default_ttl_days: u64,
        max_ttl_days: u64,
    }

    impl ApiKeyManagementConfig {
        fn new(
            auth: AuthConfig,
            environment: KeyEnvironment,
            default_ttl_days: u64,
            max_ttl_days: u64,
        ) -> Option<Self> {
            if default_ttl_days == 0
                || max_ttl_days == 0
                || default_ttl_days > max_ttl_days
                || max_ttl_days > MAX_API_KEY_TTL_DAYS
            {
                return None;
            }
            Some(Self {
                auth,
                environment,
                default_ttl_days,
                max_ttl_days,
            })
        }

        fn from_environment() -> Option<Self> {
            if !api_key_routes_configured() {
                return None;
            }
            let auth = AuthConfig::from_environment().ok().flatten()?;
            let environment = match env::var("SPUR_API_KEY_ENVIRONMENT")
                .unwrap_or_else(|_| "live".to_owned())
                .as_str()
            {
                "live" => KeyEnvironment::Live,
                "test" => KeyEnvironment::Test,
                _ => return None,
            };
            let default_ttl_days =
                environment_u64("SPUR_API_KEY_DEFAULT_TTL_DAYS", DEFAULT_API_KEY_TTL_DAYS)?;
            let max_ttl_days = environment_u64("SPUR_API_KEY_MAX_TTL_DAYS", MAX_API_KEY_TTL_DAYS)?;
            Self::new(auth, environment, default_ttl_days, max_ttl_days)
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CreateApiKeyRequest {
        name: String,
        scopes: Vec<String>,
        #[serde(default)]
        expires_at: Option<u64>,
    }

    async fn handle_api_key_management(
        route: RequestRoute,
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        config: &ApiKeyManagementConfig,
        now_epoch_seconds: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        let claims = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.jwt.as_ref())
            .and_then(|jwt| jwt.claims.as_ref());
        let decision = match auth::authorize_key_management(&config.auth, claims, now_epoch_seconds)
        {
            Ok(decision) => decision,
            Err(error) => return authorization_error_response(error),
        };
        let owner_id = decision.identity.caller_id();
        match route {
            RequestRoute::ApiKeyCreate => {
                create_api_key(request, store, config, owner_id, now_epoch_seconds).await
            }
            RequestRoute::ApiKeyList => list_api_keys(request, store, owner_id).await,
            RequestRoute::ApiKeyRevoke => {
                revoke_api_key(request, store, owner_id, now_epoch_seconds).await
            }
            _ => reserved_route_disabled_response(route),
        }
    }

    async fn create_api_key(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        config: &ApiKeyManagementConfig,
        owner_id: &str,
        now_epoch_seconds: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        create_api_key_with_generator(
            request,
            store,
            config,
            owner_id,
            now_epoch_seconds,
            generate_api_key,
        )
        .await
    }

    async fn create_api_key_with_generator<F>(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        config: &ApiKeyManagementConfig,
        owner_id: &str,
        now_epoch_seconds: u64,
        mut generator: F,
    ) -> Result<ApiGatewayResponse, Error>
    where
        F: FnMut(
            KeyEnvironment,
            &str,
            &str,
            ApiKeyScopes,
            u64,
            u64,
        ) -> Result<crate::api_keys::GeneratedApiKey, crate::api_keys::ApiKeyError>,
    {
        if request.is_base64_encoded {
            return management_error_response(400, "invalid_request");
        }
        let create = request
            .body
            .as_deref()
            .and_then(|body| serde_json::from_str::<CreateApiKeyRequest>(body).ok());
        let Some(create) = create else {
            return management_error_response(400, "invalid_request");
        };
        let scope_refs = create.scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let scopes = match ApiKeyScopes::parse(&scope_refs) {
            Ok(scopes) => scopes,
            Err(_) => return management_error_response(400, "invalid_scope"),
        };
        let default_expiry = now_epoch_seconds.checked_add(
            config
                .default_ttl_days
                .checked_mul(SECONDS_PER_DAY)
                .ok_or_else(|| lambda_error("API key default expiry overflow"))?,
        );
        let max_expiry = now_epoch_seconds.checked_add(
            config
                .max_ttl_days
                .checked_mul(SECONDS_PER_DAY)
                .ok_or_else(|| lambda_error("API key maximum expiry overflow"))?,
        );
        let (Some(default_expiry), Some(max_expiry)) = (default_expiry, max_expiry) else {
            return management_error_response(503, "key_service_unavailable");
        };
        let expires_at = create.expires_at.unwrap_or(default_expiry);
        if expires_at <= now_epoch_seconds || expires_at > max_expiry {
            return management_error_response(400, "invalid_expiry");
        }
        for attempt in 0..API_KEY_CREATE_MAX_ATTEMPTS {
            let generated = match generator(
                config.environment,
                owner_id,
                &create.name,
                scopes.clone(),
                now_epoch_seconds,
                expires_at,
            ) {
                Ok(generated) => generated,
                Err(crate::api_keys::ApiKeyError::InvalidName) => {
                    return management_error_response(400, "invalid_name");
                }
                Err(crate::api_keys::ApiKeyError::InvalidScope) => {
                    return management_error_response(400, "invalid_scope");
                }
                Err(crate::api_keys::ApiKeyError::InvalidExpiry) => {
                    return management_error_response(400, "invalid_expiry");
                }
                Err(_) => return management_error_response(503, "key_service_unavailable"),
            };
            match store
                .create_key(CreateKeyRecord::new(generated.record.clone()))
                .await
            {
                Ok(()) => {
                    let plaintext = generated.plaintext.expose_secret().to_owned();
                    let record = generated.record;
                    return one_time_secret_response(
                        201,
                        &json!({
                            "key": plaintext,
                            "key_id": record.public_id,
                            "name": record.name,
                            "scopes": record.scopes.as_strings(),
                            "created_at": record.created_at,
                            "expires_at": record.expires_at,
                        }),
                    );
                }
                Err(ApiKeyStoreError::DuplicatePublicId)
                    if attempt + 1 < API_KEY_CREATE_MAX_ATTEMPTS => {}
                Err(error) => return api_key_store_error_response(error),
            }
        }
        management_error_response(503, "key_store_unavailable")
    }

    async fn list_api_keys(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        owner_id: &str,
    ) -> Result<ApiGatewayResponse, Error> {
        let query = request.query_string_parameters.as_ref();
        let cursor = query
            .and_then(|parameters| parameters.get("cursor"))
            .map(String::as_str);
        if cursor.is_some_and(|value| value.is_empty() || value.len() > API_KEY_CURSOR_MAX_LEN) {
            return management_error_response(400, "invalid_request");
        }
        let limit = match query.and_then(|parameters| parameters.get("limit")) {
            Some(value) => match parse_api_key_list_limit(value) {
                Some(limit) => limit,
                None => return management_error_response(400, "invalid_request"),
            },
            None => API_KEY_LIST_LIMIT,
        };
        let page = match store.list_owner_keys(owner_id, cursor, limit).await {
            Ok(page) => page,
            Err(error) => return api_key_store_error_response(error),
        };
        let keys = page.keys.iter().map(api_key_metadata).collect::<Vec<_>>();
        json_response(
            200,
            &json!({ "keys": keys, "next_cursor": page.next_cursor }),
        )
    }

    fn parse_api_key_list_limit(value: &str) -> Option<usize> {
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return None;
        }
        value
            .parse()
            .ok()
            .filter(|limit| (1..=API_KEY_LIST_LIMIT).contains(limit))
    }

    fn api_key_metadata(record: &ApiKeyRecord) -> Value {
        json!({
            "key_id": record.public_id,
            "name": record.name,
            "scopes": record.scopes.as_strings(),
            "status": record.status.as_str(),
            "created_at": record.created_at,
            "expires_at": record.expires_at,
            "revoked_at": record.revoked_at,
        })
    }

    async fn revoke_api_key(
        request: &ApiGatewayRequest,
        store: &dyn ApiKeyStore,
        owner_id: &str,
        now_epoch_seconds: u64,
    ) -> Result<ApiGatewayResponse, Error> {
        let key_id = request
            .raw_path
            .as_deref()
            .or(request.path.as_deref())
            .and_then(|path| path.strip_prefix("/auth/api-keys/"))
            .filter(|key_id| valid_public_key_id(key_id));
        let Some(key_id) = key_id else {
            return management_error_response(404, "not_found");
        };
        match store.revoke_key(owner_id, key_id, now_epoch_seconds).await {
            Ok(RevokeResult::Revoked | RevokeResult::AlreadyRevoked) => {
                json_response(200, &json!({ "key_id": key_id, "status": "revoked" }))
            }
            Ok(RevokeResult::NotFound) => management_error_response(404, "not_found"),
            Err(error) => api_key_store_error_response(error),
        }
    }

    fn api_key_store_error_response(error: ApiKeyStoreError) -> Result<ApiGatewayResponse, Error> {
        match error {
            ApiKeyStoreError::OwnerLimit => management_error_response(409, "key_limit_reached"),
            ApiKeyStoreError::InvalidRequest => management_error_response(400, "invalid_request"),
            ApiKeyStoreError::DuplicatePublicId
            | ApiKeyStoreError::LeaseBusy
            | ApiKeyStoreError::Conflict
            | ApiKeyStoreError::Backend => management_error_response(503, "key_store_unavailable"),
        }
    }

    fn management_error_response(
        status_code: u16,
        code: &'static str,
    ) -> Result<ApiGatewayResponse, Error> {
        json_response(status_code, &json!({ "error": { "code": code } }))
    }

    fn valid_public_key_id(key_id: &str) -> bool {
        key_id.len() == 26
            && key_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
    }

    fn environment_u64(name: &str, default: u64) -> Option<u64> {
        match env::var(name) {
            Ok(value) => value.parse().ok(),
            Err(env::VarError::NotPresent) => Some(default),
            Err(env::VarError::NotUnicode(_)) => None,
        }
    }

    fn unix_now_seconds() -> Result<u64, Error> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| lambda_error("system clock is before Unix epoch"))
    }

    #[derive(Debug, Clone)]
    struct DiscoveryConfig {
        issuer: String,
        human_client_id: String,
        authorization_endpoint: String,
        token_endpoint: String,
        service_base_url: String,
    }

    impl DiscoveryConfig {
        fn from_environment() -> Option<Self> {
            if !environment_is_truthy("SPUR_COGNITO_AUTH_ENABLED") {
                return None;
            }
            Self::new(
                env::var("SPUR_COGNITO_ISSUER").ok()?,
                env::var("SPUR_COGNITO_HUMAN_CLIENT_ID").ok()?,
                env::var("SPUR_COGNITO_AUTHORIZATION_ENDPOINT").ok()?,
                env::var("SPUR_COGNITO_TOKEN_ENDPOINT").ok()?,
                env::var("SPUR_CONTEXT_SERVICE_BASE_URL").ok()?,
            )
        }

        fn new(
            issuer: String,
            human_client_id: String,
            authorization_endpoint: String,
            token_endpoint: String,
            service_base_url: String,
        ) -> Option<Self> {
            let bounded = [
                issuer.as_str(),
                human_client_id.as_str(),
                authorization_endpoint.as_str(),
                token_endpoint.as_str(),
                service_base_url.as_str(),
            ]
            .into_iter()
            .all(|value| {
                !value.is_empty()
                    && value.len() <= 2_048
                    && value.trim() == value
                    && !value.chars().any(char::is_control)
            });
            let secure_urls = [
                &issuer,
                &authorization_endpoint,
                &token_endpoint,
                &service_base_url,
            ]
            .into_iter()
            .all(|value| value.starts_with("https://"));
            let service_origin_is_bounded =
                https_origin(&service_base_url).is_some() && !service_base_url.contains(['?', '#']);
            let authorization_origin =
                exact_https_endpoint_origin(&authorization_endpoint, "/oauth2/authorize");
            let token_origin = exact_https_endpoint_origin(&token_endpoint, "/oauth2/token");
            let oauth_endpoints_are_consistent =
                authorization_origin.is_some() && authorization_origin == token_origin;
            if !bounded
                || !secure_urls
                || !service_origin_is_bounded
                || !oauth_endpoints_are_consistent
            {
                return None;
            }
            Some(Self {
                issuer,
                human_client_id,
                authorization_endpoint,
                token_endpoint,
                service_base_url: service_base_url.trim_end_matches('/').to_owned(),
            })
        }

        #[cfg(test)]
        fn from_contract_fixture(value: &Value) -> Option<Self> {
            Self::new(
                value.get("issuer")?.as_str()?.to_owned(),
                value.get("human_client_id")?.as_str()?.to_owned(),
                value.get("authorization_endpoint")?.as_str()?.to_owned(),
                value.get("token_endpoint")?.as_str()?.to_owned(),
                value.get("service_base_url")?.as_str()?.to_owned(),
            )
        }
    }

    fn https_origin(value: &str) -> Option<&str> {
        let authority_and_path = value.strip_prefix("https://")?;
        let authority_len = authority_and_path
            .find(['/', '?', '#'])
            .unwrap_or(authority_and_path.len());
        let authority = &authority_and_path[..authority_len];
        if !valid_dns_authority(authority) {
            return None;
        }
        Some(&value[.."https://".len() + authority_len])
    }

    fn valid_dns_authority(authority: &str) -> bool {
        !authority.is_empty()
            && authority.len() <= 253
            && authority.split('.').all(|label| {
                let bytes = label.as_bytes();
                !bytes.is_empty()
                    && bytes.len() <= 63
                    && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                    && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
                    && bytes
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            })
    }

    fn exact_https_endpoint_origin<'a>(value: &'a str, path: &str) -> Option<&'a str> {
        let origin = https_origin(value)?;
        (value.strip_prefix(origin) == Some(path)).then_some(origin)
    }

    #[derive(Debug, Serialize, PartialEq, Eq)]
    struct DiscoveryDocument {
        schema_version: u8,
        issuer: String,
        human_client_id: String,
        human_callback_url: String,
        authorization_endpoint: String,
        token_endpoint: String,
        supported_scopes: Vec<String>,
        api_key_auth_enabled: bool,
        api_key_mcp_url: String,
        api_key_management_url: String,
    }

    fn discovery_document(
        config: &DiscoveryConfig,
        api_key_auth_enabled: bool,
    ) -> Option<DiscoveryDocument> {
        let resource_server_id = env::var("SPUR_COGNITO_RESOURCE_SERVER_ID")
            .unwrap_or_else(|_| "urn:spur:context-service".to_owned());
        if resource_server_id.trim().is_empty()
            || resource_server_id.len() > 256
            || resource_server_id.chars().any(char::is_control)
        {
            return None;
        }
        Some(DiscoveryDocument {
            schema_version: 1,
            issuer: config.issuer.clone(),
            human_client_id: config.human_client_id.clone(),
            human_callback_url: HUMAN_CALLBACK_URL.to_owned(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint: config.token_endpoint.clone(),
            supported_scopes: [
                "external.index",
                "external.read",
                "external.status",
                "keys.manage",
            ]
            .into_iter()
            .map(|scope| format!("{resource_server_id}/{scope}"))
            .collect(),
            api_key_auth_enabled,
            api_key_mcp_url: format!("{}{}", config.service_base_url, auth::API_KEY_MCP_PATH),
            api_key_management_url: format!(
                "{}{}",
                config.service_base_url,
                auth::API_KEY_MANAGEMENT_PATH
            ),
        })
    }

    fn handle_reserved_route_before_body(
        request: &ApiGatewayRequest,
    ) -> Option<Result<ApiGatewayResponse, Error>> {
        let route = classify_route(request);
        match route {
            RequestRoute::ReservedUnavailable => Some(reserved_route_disabled_response(route)),
            RequestRoute::Discovery => {
                let Some(config) = DiscoveryConfig::from_environment() else {
                    return Some(reserved_route_disabled_response(route));
                };
                let enabled = ApiKeyManagementConfig::from_environment().is_some();
                Some(match discovery_document(&config, enabled) {
                    Some(document) => serde_json::to_value(document)
                        .map_err(Error::from)
                        .and_then(|value| json_response(200, &value)),
                    None => reserved_route_disabled_response(route),
                })
            }
            RequestRoute::Login => {
                let Some(config) = login_redirect_config_from_environment() else {
                    return Some(reserved_route_disabled_response(route));
                };
                Some(
                    login_redirect_response(&config, request.raw_query_string.as_deref())
                        .map_or_else(|| reserved_route_disabled_response(route), Ok),
                )
            }
            route if route.is_api_key() && !api_key_routes_configured() => {
                Some(reserved_route_disabled_response(route))
            }
            _ => None,
        }
    }

    fn login_redirect_config_from_environment() -> Option<DiscoveryConfig> {
        if !environment_is_truthy(LOGIN_REDIRECT_ENABLED_ENV) {
            return None;
        }
        DiscoveryConfig::from_environment()
    }

    fn login_redirect_response(
        config: &DiscoveryConfig,
        raw_query_string: Option<&str>,
    ) -> Option<ApiGatewayResponse> {
        let raw_query_string = raw_query_string.unwrap_or_default();
        if !safe_raw_oauth_query(raw_query_string) {
            return None;
        }

        let mut location = config.authorization_endpoint.clone();
        if !raw_query_string.is_empty() {
            location.push('?');
            location.push_str(raw_query_string);
        }
        Some(ApiGatewayResponse {
            status_code: 302,
            headers: BTreeMap::from([
                ("cache-control".to_owned(), "no-store".to_owned()),
                ("content-length".to_owned(), "0".to_owned()),
                ("location".to_owned(), location),
                ("pragma".to_owned(), "no-cache".to_owned()),
                ("referrer-policy".to_owned(), "no-referrer".to_owned()),
                ("x-content-type-options".to_owned(), "nosniff".to_owned()),
            ]),
            body: String::new(),
            is_base64_encoded: false,
        })
    }

    fn safe_raw_oauth_query(raw_query_string: &str) -> bool {
        if raw_query_string.len() > LOGIN_REDIRECT_MAX_RAW_QUERY_LEN {
            return false;
        }

        let bytes = raw_query_string.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'%' {
                let (Some(&high), Some(&low)) = (bytes.get(index + 1), bytes.get(index + 2)) else {
                    return false;
                };
                let (Some(high), Some(low)) = (hex_value(high), hex_value(low)) else {
                    return false;
                };
                let encoded = (high << 4) | low;
                if encoded.is_ascii_control() {
                    return false;
                }
                index += 3;
                continue;
            }
            if !(byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                        | b'/'
                        | b'?'
                ))
            {
                return false;
            }
            index += 1;
        }
        true
    }

    const fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    fn reserved_route_disabled_response(_route: RequestRoute) -> Result<ApiGatewayResponse, Error> {
        json_response(404, &json!({ "error": { "code": "route_unavailable" } }))
    }

    fn api_key_auth_enabled() -> bool {
        environment_is_truthy("SPUR_API_KEY_AUTH_ENABLED")
    }

    fn api_key_store_configured() -> bool {
        env::var("SPUR_CONTEXT_API_KEYS_TABLE")
            .ok()
            .is_some_and(|name| !name.trim().is_empty() && name.len() <= 255)
    }

    fn api_key_routes_configured() -> bool {
        api_key_auth_enabled()
            && api_key_store_configured()
            && DiscoveryConfig::from_environment().is_some()
    }

    fn environment_is_truthy(name: &str) -> bool {
        matches!(
            env::var(name).ok().as_deref().map(str::trim),
            Some("1") | Some("true") | Some("TRUE") | Some("yes")
        )
    }

    pub async fn route_index_status_control_plane(
        args: &Value,
        jobs: &dyn JobStore,
        checker: &dyn mcp::ExecutionStatusChecker,
        caller_id: &str,
    ) -> Result<Value, McpHandlerError> {
        mcp::route_index_status_for_caller(args, jobs, Some(checker), caller_id).await
    }

    fn parse_tool_request(request: &ApiGatewayRequest) -> Result<ToolRequest, McpHandlerError> {
        if request.is_base64_encoded {
            return Err(McpHandlerError::InvalidParams(
                "base64-encoded API Gateway bodies are not supported".to_owned(),
            ));
        }
        let body = request
            .body
            .as_deref()
            .ok_or_else(|| McpHandlerError::InvalidParams("missing request body".to_owned()))?;
        if let Some(tool) = routed_tool_name(request) {
            let value: Value = serde_json::from_str(body).map_err(|error| {
                McpHandlerError::InvalidParams(format!(
                    "failed to parse request JSON body: {error}"
                ))
            })?;
            let args = value.get("args").cloned().unwrap_or(value);
            return Ok(ToolRequest {
                tool: tool.to_owned(),
                args,
            });
        }
        serde_json::from_str(body).map_err(|error| {
            McpHandlerError::InvalidParams(format!("failed to parse request JSON body: {error}"))
        })
    }

    fn routed_tool_name(request: &ApiGatewayRequest) -> Option<&'static str> {
        let path = request.raw_path.as_deref().or(request.path.as_deref())?;
        match path.trim_end_matches('/').rsplit('/').next() {
            Some("index") => Some("external_index"),
            Some("index_status") => Some("external_index_status"),
            _ => None,
        }
    }

    #[cfg(test)]
    fn is_oauth_route(request: &ApiGatewayRequest) -> bool {
        matches!(classify_route(request), RequestRoute::OAuth)
    }

    fn api_key_context(
        request: &ApiGatewayRequest,
    ) -> Result<crate::api_key_authorizer::ApiKeyAuthContext, AuthFailure> {
        let value = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.lambda.as_ref())
            .ok_or(AuthFailure::MissingContext)?;
        crate::api_key_authorizer::ApiKeyAuthContext::from_value(value)
            .map_err(|_| AuthFailure::InvalidApiKeyContext)
    }

    #[cfg(test)]
    fn authorize_api_key_request(
        request: &ApiGatewayRequest,
        tool: &str,
    ) -> Result<AuthDecision, AuthFailure> {
        let context = api_key_context(request)?;
        auth::authorize_api_key_tool(tool, Some(&context))
    }

    fn authorize_oauth_request_now(
        request: &ApiGatewayRequest,
        tool: &str,
        config: &AuthConfig,
    ) -> Result<AuthDecision, AuthFailure> {
        let claims = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.jwt.as_ref())
            .and_then(|jwt| jwt.claims.as_ref());
        auth::authorize_oauth_tool_now(config, tool, claims)
    }

    #[cfg(test)]
    fn authorize_oauth_request(
        request: &ApiGatewayRequest,
        tool: &str,
        config: &AuthConfig,
        now_epoch_seconds: u64,
    ) -> Result<AuthDecision, AuthFailure> {
        let claims = request
            .request_context
            .as_ref()
            .and_then(|context| context.authorizer.as_ref())
            .and_then(|authorizer| authorizer.jwt.as_ref())
            .and_then(|jwt| jwt.claims.as_ref());
        auth::authorize_oauth_tool(config, tool, claims, now_epoch_seconds)
    }

    fn catalog_resolver() -> Result<MutexGuard<'static, Option<CatalogCacheEntry>>, Error> {
        CATALOG_RESOLVER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|error| lambda_error(format!("catalog resolver cache is poisoned: {error}")))
    }

    fn initialized_catalog<'a>(
        guard: &'a mut MutexGuard<'static, Option<CatalogCacheEntry>>,
        prepared_catalog: &PreparedCatalog,
    ) -> Result<&'a CatalogResolver, Error> {
        let cached_dsn = guard.as_ref().map(|entry| entry.catalog_dsn.as_str());
        let cached_etag = guard
            .as_ref()
            .and_then(|entry| entry.catalog_etag.as_deref());
        let catalog_etag = prepared_catalog.catalog_etag.as_deref();

        if should_initialize_catalog(
            cached_dsn,
            cached_etag,
            &prepared_catalog.cache_key,
            catalog_etag,
        ) {
            let conn = match &prepared_catalog.source {
                PreparedCatalogSource::FrozenSnapshot {
                    local_path,
                    data_path,
                } => {
                    catalog::connect_frozen_snapshot(local_path, data_path).map_err(Error::from)?
                }
                PreparedCatalogSource::Direct { catalog_dsn } => {
                    catalog::connect_ducklake(catalog_dsn).map_err(Error::from)?
                }
            };
            **guard = Some(CatalogCacheEntry {
                catalog_dsn: prepared_catalog.cache_key.clone(),
                catalog_etag: prepared_catalog.catalog_etag.clone(),
                resolver: CatalogResolver::from_connection(conn),
            });
        }
        guard
            .as_ref()
            .map(|entry| &entry.resolver)
            .ok_or_else(|| lambda_error("catalog resolver cache did not initialize"))
    }

    /// Runs one synchronous DuckDB operation while holding the process catalog
    /// mutex. The closure must return owned data, so no catalog borrow or mutex
    /// guard can cross an I/O await in the caller.
    fn with_initialized_catalog<T: 'static>(
        prepared_catalog: &PreparedCatalog,
        operation: impl FnOnce(&CatalogResolver) -> T,
    ) -> Result<T, Error> {
        let mut catalog_guard = catalog_resolver()?;
        let catalog = initialized_catalog(&mut catalog_guard, prepared_catalog)?;
        Ok(operation(catalog))
    }

    async fn prepare_catalog() -> Result<Option<PreparedCatalog>, Error> {
        let catalog_dsn = catalog_dsn()?;
        prepare_catalog_source(catalog_dsn).await
    }

    async fn prepare_catalog_source(catalog_dsn: String) -> Result<Option<PreparedCatalog>, Error> {
        if let Some(uri) = parse_s3_uri(&catalog_dsn)? {
            if uri.key.ends_with(".json") {
                let Some(pointer) = download_snapshot_pointer(&uri).await? else {
                    return Ok(None);
                };
                let Some(snapshot_uri) = parse_s3_uri(&pointer.manifest.snapshot_uri)? else {
                    return Err(lambda_error(format!(
                        "snapshot pointer must reference an S3 snapshot URI, got `{}`",
                        pointer.manifest.snapshot_uri
                    )));
                };
                let cache_token = snapshot_pointer_cache_token(
                    pointer.pointer_etag.as_deref(),
                    &pointer.manifest,
                );
                let local_path =
                    local_snapshot_path(&pointer.manifest.snapshot_uri, Some(&cache_token))?;
                ensure_local_snapshot(&local_path, &pointer.manifest.sha256, || {
                    download_catalog_snapshot(&snapshot_uri, &local_path)
                })
                .await?;
                return Ok(Some(PreparedCatalog {
                    cache_key: snapshot_pointer_cache_key(
                        &catalog_dsn,
                        pointer.pointer_etag.as_deref(),
                        &pointer.manifest,
                    ),
                    catalog_etag: pointer.pointer_etag,
                    source: PreparedCatalogSource::FrozenSnapshot {
                        local_path,
                        data_path: pointer.manifest.data_path,
                    },
                }));
            }

            let catalog_etag = catalog_etag(&catalog_dsn).await?;
            let data_path = catalog_data_path(&catalog_dsn)?;
            let local_path = local_snapshot_path(&catalog_dsn, catalog_etag.as_deref())?;
            if !local_path.is_file() {
                download_catalog_snapshot(&uri, &local_path).await?;
            }
            return Ok(Some(PreparedCatalog {
                cache_key: format!("{catalog_dsn}\n{data_path}"),
                catalog_etag,
                source: PreparedCatalogSource::FrozenSnapshot {
                    local_path,
                    data_path,
                },
            }));
        }

        let catalog_etag = None;
        if is_postgres_catalog_dsn(&catalog_dsn) {
            return Err(lambda_error(
            "serving requires SPUR_CATALOG_S3_URI to point at a frozen DuckLake snapshot; refusing to connect to Postgres",
        ));
        }

        Ok(Some(PreparedCatalog {
            cache_key: catalog_dsn.clone(),
            catalog_etag,
            source: PreparedCatalogSource::Direct { catalog_dsn },
        }))
    }

    fn should_initialize_catalog(
        cached_dsn: Option<&str>,
        cached_etag: Option<&str>,
        catalog_dsn: &str,
        current_etag: Option<&str>,
    ) -> bool {
        let Some(cached_dsn) = cached_dsn else {
            return true;
        };
        if cached_dsn != catalog_dsn {
            return true;
        }
        if !catalog::is_remote_catalog(catalog_dsn) {
            return false;
        }
        match (cached_etag, current_etag) {
            (Some(cached), Some(current)) => cached != current,
            _ => true,
        }
    }

    fn catalog_dsn() -> Result<String, Error> {
        if let Ok(value) = env::var("SPUR_CATALOG_S3_URI") {
            if !value.trim().is_empty() {
                return Ok(value);
            }
        }

        let catalog_dsn = env::var("SPUR_CATALOG_DSN").map_err(|error| {
            lambda_error(format!(
                "SPUR_CATALOG_S3_URI environment variable is required for serving: {error}"
            ))
        })?;
        if is_postgres_catalog_dsn(&catalog_dsn) {
            return Err(lambda_error(
            "SPUR_CATALOG_S3_URI must point at the frozen serving snapshot; SPUR_CATALOG_DSN Postgres catalogs are ingest-only",
        ));
        }
        Ok(catalog_dsn)
    }

    async fn catalog_etag(catalog_dsn: &str) -> Result<Option<String>, Error> {
        let Some(uri) = parse_s3_uri(catalog_dsn)? else {
            return Ok(None);
        };
        let output = aws_clients()
            .s3
            .head_object()
            .bucket(uri.bucket)
            .key(uri.key)
            .send()
            .await
            .map_err(|error| lambda_error(format!("failed to read catalog ETag: {error}")))?;
        Ok(output.e_tag().map(str::to_owned))
    }

    struct SnapshotPointerDownload {
        manifest: catalog::FrozenSnapshotManifest,
        pointer_etag: Option<String>,
    }

    async fn download_snapshot_pointer(
        uri: &S3Uri,
    ) -> Result<Option<SnapshotPointerDownload>, Error> {
        let output = match aws_clients()
            .s3
            .get_object()
            .bucket(&uri.bucket)
            .key(&uri.key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let error = anyhow::Error::new(error);
                if catalog::is_s3_not_found_error(&error) {
                    return Ok(None);
                }
                return Err(lambda_error(format!(
                    "failed to download catalog pointer: {error:#}"
                )));
            }
        };
        let pointer_etag = output.e_tag().map(str::to_owned);
        let bytes = output.body.collect().await.map_err(|error| {
            lambda_error(format!(
                "failed to read catalog pointer download body: {error}"
            ))
        })?;
        let bytes = bytes.into_bytes();
        let manifest = catalog::FrozenSnapshotManifest::from_json_slice(bytes.as_ref())
            .map_err(Error::from)?;
        Ok(Some(SnapshotPointerDownload {
            manifest,
            pointer_etag,
        }))
    }

    fn snapshot_pointer_cache_key(
        pointer_uri: &str,
        pointer_etag: Option<&str>,
        manifest: &catalog::FrozenSnapshotManifest,
    ) -> String {
        format!(
            "pointer={pointer_uri}\netag={}\ngeneration={}\nsnapshot={}\nsha256={}",
            pointer_etag.unwrap_or("<missing>"),
            manifest.generation,
            manifest.snapshot_uri,
            manifest.sha256
        )
    }

    fn snapshot_pointer_cache_token(
        pointer_etag: Option<&str>,
        manifest: &catalog::FrozenSnapshotManifest,
    ) -> String {
        format!(
            "{}:{}:{}",
            pointer_etag.unwrap_or("<missing>"),
            manifest.generation,
            manifest.sha256
        )
    }

    async fn ensure_local_snapshot<Download, DownloadFuture>(
        local_path: &Path,
        expected_sha256: &str,
        download: Download,
    ) -> Result<(), Error>
    where
        Download: FnOnce() -> DownloadFuture,
        DownloadFuture: Future<Output = Result<(), Error>>,
    {
        // Snapshot preparation happens before the DuckDB catalog mutex is taken.
        // Serialize cache repair separately so concurrent cold invokes cannot race
        // while replacing the same local snapshot path.
        let _cache_guard = SNAPSHOT_CACHE_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;

        if local_path.is_file() && verify_local_snapshot_hash(local_path, expected_sha256).is_ok() {
            return Ok(());
        }

        evict_local_snapshot(local_path).await?;
        download().await?;
        if let Err(error) = verify_local_snapshot_hash(local_path, expected_sha256) {
            evict_local_snapshot(local_path).await?;
            return Err(error);
        }
        Ok(())
    }

    async fn evict_local_snapshot(local_path: &Path) -> Result<(), Error> {
        match tokio::fs::remove_file(local_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(lambda_error(format!(
                "failed to evict invalid catalog snapshot `{}`: {error}",
                local_path.display()
            ))),
        }
    }

    fn verify_local_snapshot_hash(local_path: &Path, expected_sha256: &str) -> Result<(), Error> {
        let bytes = fs::read(local_path).map_err(|error| {
            lambda_error(format!(
                "failed to read cached catalog snapshot `{}`: {error}",
                local_path.display()
            ))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if actual != expected_sha256 {
            return Err(lambda_error(format!(
            "cached catalog snapshot `{}` sha256 mismatch: expected {expected_sha256}, got {actual}",
            local_path.display()
        )));
        }
        Ok(())
    }

    fn catalog_data_path(snapshot_uri: &str) -> Result<String, Error> {
        if let Ok(path) = env::var("SPUR_CONTEXT_DUCKLAKE_DATA_PATH") {
            if !path.trim().is_empty() {
                return Ok(path);
            }
        }

        infer_data_path_from_snapshot_uri(snapshot_uri).ok_or_else(|| {
        lambda_error(
            "SPUR_CONTEXT_DUCKLAKE_DATA_PATH must be set when the frozen snapshot URI does not include /gold/catalog-snapshot/",
        )
    })
    }

    fn infer_data_path_from_snapshot_uri(snapshot_uri: &str) -> Option<String> {
        let marker = "/gold/catalog-snapshot/";
        let prefix = snapshot_uri.split_once(marker)?.0;
        Some(format!("{prefix}/gold/data/"))
    }

    fn local_snapshot_path(
        catalog_dsn: &str,
        catalog_etag: Option<&str>,
    ) -> Result<PathBuf, Error> {
        let mut hasher = Sha256::new();
        hasher.update(catalog_dsn.as_bytes());
        if let Some(etag) = catalog_etag {
            hasher.update(b"\0");
            hasher.update(etag.as_bytes());
        }
        let digest = hasher.finalize();
        let suffix = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let dir = env::temp_dir().join("spur-context-service-catalog");
        fs::create_dir_all(&dir).map_err(|error| {
            lambda_error(format!(
                "failed to create catalog snapshot cache dir `{}`: {error}",
                dir.display()
            ))
        })?;
        Ok(dir.join(format!("catalog-{suffix}.ducklake")))
    }

    async fn download_catalog_snapshot(uri: &S3Uri, local_path: &Path) -> Result<(), Error> {
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                lambda_error(format!(
                    "failed to create catalog snapshot dir `{}`: {error}",
                    parent.display()
                ))
            })?;
        }

        let output = aws_clients()
            .s3
            .get_object()
            .bucket(&uri.bucket)
            .key(&uri.key)
            .send()
            .await
            .map_err(|error| {
                lambda_error(format!("failed to download catalog snapshot: {error}"))
            })?;
        let bytes = output.body.collect().await.map_err(|error| {
            lambda_error(format!(
                "failed to read catalog snapshot download body: {error}"
            ))
        })?;

        let tmp_path = local_path.with_file_name(format!(
            ".{}.{}.tmp",
            local_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("catalog.ducklake"),
            std::process::id()
        ));
        tokio::fs::write(&tmp_path, bytes.into_bytes())
            .await
            .map_err(|error| {
                lambda_error(format!(
                    "failed to write catalog snapshot `{}`: {error}",
                    tmp_path.display()
                ))
            })?;
        tokio::fs::rename(&tmp_path, local_path)
            .await
            .map_err(|error| {
                lambda_error(format!(
                    "failed to install catalog snapshot `{}`: {error}",
                    local_path.display()
                ))
            })
    }

    struct S3Uri {
        bucket: String,
        key: String,
    }

    fn parse_s3_uri(uri: &str) -> Result<Option<S3Uri>, Error> {
        let Some(without_scheme) = uri.strip_prefix("s3://") else {
            return Ok(None);
        };
        let (bucket, key) = without_scheme.split_once('/').ok_or_else(|| {
            lambda_error(format!("S3 catalog URI must include bucket and key: {uri}"))
        })?;
        if bucket.is_empty() || key.is_empty() {
            return Err(lambda_error(format!(
                "S3 catalog URI must include bucket and key: {uri}"
            )));
        }
        Ok(Some(S3Uri {
            bucket: bucket.to_owned(),
            key: key.to_owned(),
        }))
    }

    fn is_postgres_catalog_dsn(catalog_dsn: &str) -> bool {
        let dsn = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
        dsn.starts_with("postgres:")
            || dsn.starts_with("postgresql:")
            || dsn.starts_with("postgresql://")
    }

    fn job_store() -> DynamoDbJobStore {
        DynamoDbJobStore::new(aws_clients().dynamodb.clone())
    }

    fn api_key_store() -> DynamoDbApiKeyStore {
        DynamoDbApiKeyStore::new(aws_clients().dynamodb.clone())
    }

    fn status_checker() -> mcp::SfnExecutionStatusChecker {
        mcp::SfnExecutionStatusChecker::new(aws_clients().sfn.clone())
    }

    fn sfn_client() -> Result<SfnIndexExecutionStarter, Error> {
        let client = aws_clients().sfn.clone();
        Ok(SfnIndexExecutionStarter {
            client,
            state_machine_arn: env::var("SPUR_INDEX_STATE_MACHINE_ARN").map_err(|error| {
                lambda_error(format!(
                    "SPUR_INDEX_STATE_MACHINE_ARN environment variable is required: {error}"
                ))
            })?,
        })
    }

    /// Run one bounded drainer invocation using the production DynamoDB job store
    /// and Step Functions starter. This is the correctness path that dispatches
    /// queued index jobs under configured running caps.
    ///
    /// It is called by the EventBridge-scheduled correctness trigger and as a
    /// best-effort kick after a successful enqueue (see [`handler`]). Kick errors
    /// are logged and do not affect the admission response.
    pub async fn drain_queued_jobs() -> Result<drainer::DrainSummary, Error> {
        let jobs = job_store();
        let starter = sfn_client()?;
        let checker = status_checker();
        let config = mcp::index_queue_config();
        let limits = mcp::index_drainer_limits();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        Ok(drainer::Drainer::new(&jobs, &starter, config)
            .with_limits(limits.max_dispatches_per_run, limits.scan_limit_per_shard)
            .with_rotation_interval_secs(limits.rotation_interval_secs)
            .with_checker(&checker)
            .drain(now_secs)
            .await)
    }

    /// Whether an `external_index` response represents a job accepted into the
    /// queue (status = "queued"). Used to decide whether to kick the drainer.
    fn is_queued_job_response(value: &Value) -> bool {
        value.get("status").and_then(Value::as_str) == Some("queued")
    }

    /// Best-effort drainer kick. Logs failures but never propagates them — the
    /// scheduled EventBridge drainer is the correctness fallback.
    async fn kick_drainer() {
        if let Err(error) = drain_queued_jobs().await {
            eprintln!("[lambda] best-effort drainer kick failed: {error}");
        }
    }

    #[derive(Debug, Clone)]
    struct AwsClients {
        dynamodb: aws_sdk_dynamodb::Client,
        s3: aws_sdk_s3::Client,
        sfn: aws_sdk_sfn::Client,
    }

    fn aws_clients() -> &'static AwsClients {
        AWS_CLIENTS.get_or_init(|| AwsClients {
            dynamodb: dynamodb_client_from_env(),
            s3: s3_client_from_env(),
            sfn: sfn_client_from_env(),
        })
    }

    fn sfn_client_from_env() -> aws_sdk_sfn::Client {
        let region = env::var("AWS_REGION")
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_owned());
        let mut config = aws_sdk_sfn::Config::builder()
            .behavior_version(aws_sdk_sfn::config::BehaviorVersion::latest())
            .region(aws_sdk_sfn::config::Region::new(region));

        if let (Ok(access_key), Ok(secret_key)) = (
            env::var("AWS_ACCESS_KEY_ID"),
            env::var("AWS_SECRET_ACCESS_KEY"),
        ) {
            config = config.credentials_provider(aws_sdk_sfn::config::Credentials::new(
                access_key,
                secret_key,
                env::var("AWS_SESSION_TOKEN").ok(),
                None,
                "lambda-env",
            ));
        }

        aws_sdk_sfn::Client::from_conf(config.build())
    }

    fn dynamodb_client_from_env() -> aws_sdk_dynamodb::Client {
        let region = env::var("AWS_REGION")
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_owned());
        let mut config = aws_sdk_dynamodb::Config::builder()
            .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
            .region(aws_sdk_dynamodb::config::Region::new(region));

        if let (Ok(access_key), Ok(secret_key)) = (
            env::var("AWS_ACCESS_KEY_ID"),
            env::var("AWS_SECRET_ACCESS_KEY"),
        ) {
            config = config.credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
                access_key,
                secret_key,
                env::var("AWS_SESSION_TOKEN").ok(),
                None,
                "lambda-env",
            ));
        }

        aws_sdk_dynamodb::Client::from_conf(config.build())
    }

    fn s3_client_from_env() -> aws_sdk_s3::Client {
        let region = env::var("AWS_REGION")
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_owned());
        let mut config = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new(region));

        if let (Ok(access_key), Ok(secret_key)) = (
            env::var("AWS_ACCESS_KEY_ID"),
            env::var("AWS_SECRET_ACCESS_KEY"),
        ) {
            config = config.credentials_provider(aws_sdk_s3::config::Credentials::new(
                access_key,
                secret_key,
                env::var("AWS_SESSION_TOKEN").ok(),
                None,
                "lambda-env",
            ));
        }

        aws_sdk_s3::Client::from_conf(config.build())
    }

    /// When set truthy, mutating tools (`external_index`/`external_index_status`)
    /// no longer require an authenticated caller and fall back to a shared anonymous
    /// identity. Intended for internal-team / trusted-network deployments where the
    /// HTTP API route is `NONE` (no authorizer injects a caller). Secure-by-default:
    /// off unless explicitly enabled.
    const ALLOW_ANONYMOUS_MUTATIONS_ENV: &str = "SPUR_CONTEXT_ALLOW_ANONYMOUS_MUTATIONS";
    /// Shared caller id used for anonymous mutations. All anonymous callers share
    /// this bucket, so the existing per-caller rate limit / active-job cap still
    /// apply (collectively) rather than being bypassed entirely.
    #[cfg(test)]
    const ANONYMOUS_CALLER_ID: &str = "anonymous-internal";

    fn anonymous_mutations_allowed() -> bool {
        matches!(
            env::var(ALLOW_ANONYMOUS_MUTATIONS_ENV)
                .ok()
                .as_deref()
                .map(str::trim),
            Some("1") | Some("true") | Some("TRUE") | Some("yes")
        )
    }

    struct SfnIndexExecutionStarter {
        client: aws_sdk_sfn::Client,
        state_machine_arn: String,
    }

    impl mcp::IndexExecutionStarter for SfnIndexExecutionStarter {
        fn start_execution<'a>(
            &'a self,
            request: mcp::IndexExecutionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<String, McpHandlerError>> + Send + 'a>> {
            Box::pin(async move {
                let input = serde_json::to_string(&request.input).map_err(|error| {
                    McpHandlerError::Internal(format!(
                        "external_index StartExecution input serialization failed: {error}"
                    ))
                })?;
                let output = self
                    .client
                    .start_execution()
                    .state_machine_arn(self.state_machine_arn.clone())
                    .name(request.name)
                    .input(input)
                    .send()
                    .await
                    .map_err(|error| {
                        McpHandlerError::Internal(format!(
                            "external_index StartExecution failed: {error}"
                        ))
                    })?;
                Ok(output.execution_arn().to_owned())
            })
        }
    }

    fn tool_error_response(error: McpHandlerError) -> Result<ApiGatewayResponse, Error> {
        json_response(
            200,
            &json!({
                "error": {
                    "code": error.json_rpc_code(),
                    "message": error.to_string()
                }
            }),
        )
    }

    fn auth_error_response(error: McpHandlerError) -> Result<ApiGatewayResponse, Error> {
        json_response(
            401,
            &json!({
                "error": {
                    "code": error.json_rpc_code(),
                    "message": error.to_string()
                }
            }),
        )
    }

    fn authorization_error_response(error: AuthFailure) -> Result<ApiGatewayResponse, Error> {
        let code = if error.status_code() == 401 {
            "authentication_failed"
        } else {
            "authorization_failed"
        };
        json_response(
            error.status_code(),
            &json!({
                "error": {
                    "code": code,
                    "reason": error.reason(),
                }
            }),
        )
    }

    fn one_time_secret_response(
        status_code: u16,
        value: &Value,
    ) -> Result<ApiGatewayResponse, Error> {
        let mut response = json_response(status_code, value)?;
        response
            .headers
            .insert("cache-control".to_owned(), "no-store".to_owned());
        response
            .headers
            .insert("pragma".to_owned(), "no-cache".to_owned());
        Ok(response)
    }

    fn lambda_error(message: impl Into<String>) -> Error {
        Box::new(std::io::Error::other(message.into()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        static LOGIN_ENV_LOCK: Mutex<()> = Mutex::new(());

        fn hybrid_auth_fixture() -> Value {
            serde_json::from_str(include_str!("../tests/fixtures/hybrid-auth-contract.json"))
                .expect("hybrid auth contract fixture should be valid JSON")
        }

        fn api_key_auth_fixture() -> Value {
            serde_json::from_str(include_str!("../tests/fixtures/api-key-auth-contract.json"))
                .expect("API-key auth contract fixture should be valid JSON")
        }

        fn poc_external_index_fixture() -> &'static str {
            include_str!(
            "../../../infra/spur-context-service/poc/fixtures/external-index-validation-only.json"
        )
        }

        fn fixture_cases<'a>(fixture: &'a Value, key: &str) -> &'a [Value] {
            fixture[key]
                .as_array()
                .unwrap_or_else(|| panic!("fixture field {key:?} should be an array"))
        }

        fn fixture_str<'a>(value: &'a Value, key: &str) -> &'a str {
            value[key]
                .as_str()
                .unwrap_or_else(|| panic!("fixture field {key:?} should be a string"))
        }

        #[test]
        fn api_key_fixture_reserves_exact_routes_and_publishes_bounded_discovery() {
            let fixture = api_key_auth_fixture();

            for case in fixture_cases(&fixture, "route_cases") {
                let route = auth::classify_route(
                    Some(fixture_str(case, "path")),
                    case.get("method").and_then(Value::as_str),
                );
                assert_eq!(route.as_str(), fixture_str(case, "expected"));
            }

            let config = DiscoveryConfig::from_contract_fixture(&fixture["discovery_config"])
                .expect("fixture discovery configuration should be valid");
            let document = discovery_document(&config, true)
                .expect("configured Cognito discovery should be available");
            let document =
                serde_json::to_value(document).expect("discovery document should serialize");
            assert_eq!(
                document["issuer"],
                "https://cognito-idp.ap-southeast-5.amazonaws.com/ap-southeast-5_fixture"
            );
            assert_eq!(
                document["authorization_endpoint"],
                "https://auth.context.getspur.dev/oauth2/authorize"
            );
            assert_eq!(
                document["token_endpoint"],
                "https://auth.context.getspur.dev/oauth2/token"
            );
            assert_eq!(
                document["api_key_mcp_url"],
                "https://context.getspur.dev/mcp/api-key"
            );
            assert_eq!(document, fixture["discovery_document"]);
        }

        #[test]
        fn api_key_discovery_rejects_insecure_or_mismatched_endpoint_configuration() {
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth-a.example/oauth2/authorize".to_owned(),
                "https://auth-b.example/oauth2/token".to_owned(),
                "https://context.example".to_owned(),
            )
            .is_none());
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth.example/oauth2/authorize".to_owned(),
                "https://auth.example/oauth2/token".to_owned(),
                "x".to_owned(),
            )
            .is_none());
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "http://auth.example/oauth2/authorize".to_owned(),
                "https://auth.example/oauth2/token".to_owned(),
                "https://context.example".to_owned(),
            )
            .is_none());
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth.example/redirector".to_owned(),
                "https://auth.example/oauth2/token".to_owned(),
                "https://context.example".to_owned(),
            )
            .is_none());
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth.example/oauth2/authorize".to_owned(),
                "https://auth.example/not-a-token-endpoint".to_owned(),
                "https://context.example".to_owned(),
            )
            .is_none());
            assert!(DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth.example\\attacker.example/oauth2/authorize".to_owned(),
                "https://auth.example\\attacker.example/oauth2/token".to_owned(),
                "https://context.example".to_owned(),
            )
            .is_none());
        }

        fn login_discovery_config() -> DiscoveryConfig {
            DiscoveryConfig::new(
                "https://issuer.example/pool".to_owned(),
                "human-client".to_owned(),
                "https://auth.context.example/oauth2/authorize".to_owned(),
                "https://auth.context.example/oauth2/token".to_owned(),
                "https://context.example".to_owned(),
            )
            .expect("login discovery configuration should be valid")
        }

        #[test]
        fn login_redirect_preserves_the_exact_safe_raw_query_on_the_cognito_endpoint() {
            let raw_query = concat!(
                "response_type=code&client_id=human-client&",
                "redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback&",
                "state=a%2Bb%3D&scope=openid%20external.read"
            );

            let response = login_redirect_response(&login_discovery_config(), Some(raw_query))
                .expect("safe OAuth authorization query should redirect");

            assert_eq!(response.status_code, 302);
            assert_eq!(
            response.headers.get("location").map(String::as_str),
            Some(
                "https://auth.context.example/oauth2/authorize?response_type=code&client_id=human-client&redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback&state=a%2Bb%3D&scope=openid%20external.read"
            )
        );
            assert_eq!(
                response.headers.get("cache-control").map(String::as_str),
                Some("no-store")
            );
            assert_eq!(
                response.headers.get("pragma").map(String::as_str),
                Some("no-cache")
            );
            assert_eq!(
                response.headers.get("referrer-policy").map(String::as_str),
                Some("no-referrer")
            );
            assert_eq!(
                response.headers.get("content-length").map(String::as_str),
                Some("0")
            );
            assert!(response.body.is_empty());
            assert!(!response.is_base64_encoded);
        }

        #[test]
        fn login_redirect_never_replaces_the_validated_cognito_authority() {
            let response = login_redirect_response(
                &login_discovery_config(),
                Some("//attacker.example&redirect_uri=https%3A%2F%2Fattacker.example%2Fcallback"),
            )
            .expect("a query-like authority string remains data after the question mark");

            assert_eq!(
            response.headers.get("location").map(String::as_str),
            Some(
                "https://auth.context.example/oauth2/authorize?//attacker.example&redirect_uri=https%3A%2F%2Fattacker.example%2Fcallback"
            )
        );
        }

        #[test]
        fn login_redirect_rejects_ambiguous_injected_or_oversized_raw_queries() {
            for raw_query in [
                "state=literal\r\ninjected:true",
                "state=encoded%0d%0ainjected",
                "state=fragment#https://attacker.example",
                "state=raw space",
                "state=bad%encoding",
            ] {
                assert!(
                    login_redirect_response(&login_discovery_config(), Some(raw_query)).is_none(),
                    "unsafe raw query must fail closed: {raw_query:?}"
                );
            }
            let oversized = format!("state={}", "a".repeat(8_193));
            assert!(login_redirect_response(&login_discovery_config(), Some(&oversized)).is_none());
        }

        #[test]
        fn login_redirect_configuration_is_explicit_and_fail_closed() {
            let _environment_guard = LOGIN_ENV_LOCK.lock().expect("login environment lock");
            let _cognito_enabled = EnvVarRestore::set("SPUR_COGNITO_AUTH_ENABLED", "1");
            let _issuer = EnvVarRestore::set("SPUR_COGNITO_ISSUER", "https://issuer.example/pool");
            let _client = EnvVarRestore::set("SPUR_COGNITO_HUMAN_CLIENT_ID", "human-client");
            let _authorization = EnvVarRestore::set(
                "SPUR_COGNITO_AUTHORIZATION_ENDPOINT",
                "https://auth.context.example/oauth2/authorize",
            );
            let _token = EnvVarRestore::set(
                "SPUR_COGNITO_TOKEN_ENDPOINT",
                "https://auth.context.example/oauth2/token",
            );
            let _service =
                EnvVarRestore::set("SPUR_CONTEXT_SERVICE_BASE_URL", "https://context.example");

            {
                let _facade_disabled =
                    EnvVarRestore::set("SPUR_CONTEXT_LOGIN_REDIRECT_ENABLED", "0");
                assert!(login_redirect_config_from_environment().is_none());
            }
            {
                let _facade_enabled =
                    EnvVarRestore::set("SPUR_CONTEXT_LOGIN_REDIRECT_ENABLED", "1");
                let _malformed_endpoint = EnvVarRestore::set(
                    "SPUR_COGNITO_AUTHORIZATION_ENDPOINT",
                    "https://attacker.example/redirector",
                );
                assert!(login_redirect_config_from_environment().is_none());
            }
        }

        #[test]
        fn login_facade_redirects_before_parsing_or_forwarding_the_request_body() {
            let _environment_guard = LOGIN_ENV_LOCK.lock().expect("login environment lock");
            let _cognito_enabled = EnvVarRestore::set("SPUR_COGNITO_AUTH_ENABLED", "1");
            let _facade_enabled = EnvVarRestore::set("SPUR_CONTEXT_LOGIN_REDIRECT_ENABLED", "1");
            let _issuer = EnvVarRestore::set("SPUR_COGNITO_ISSUER", "https://issuer.example/pool");
            let _client = EnvVarRestore::set("SPUR_COGNITO_HUMAN_CLIENT_ID", "human-client");
            let _authorization = EnvVarRestore::set(
                "SPUR_COGNITO_AUTHORIZATION_ENDPOINT",
                "https://auth.context.example/oauth2/authorize",
            );
            let _token = EnvVarRestore::set(
                "SPUR_COGNITO_TOKEN_ENDPOINT",
                "https://auth.context.example/oauth2/token",
            );
            let _service =
                EnvVarRestore::set("SPUR_CONTEXT_SERVICE_BASE_URL", "https://context.example");
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/auth/login",
                "rawQueryString": "response_type=code&state=opaque",
                "body": "not-json-and-must-not-be-read",
                "headers": {
                    "authorization": "Bearer must-not-be-forwarded",
                    "cookie": "session=must-not-be-forwarded"
                },
                "requestContext": { "http": { "method": "GET" } }
            }))
            .expect("login request should deserialize");

            let response = handle_reserved_route_before_body(&request)
                .expect("the exact login route should return before body parsing")
                .expect("the login redirect response should be infallible");

            assert_eq!(response.status_code, 302);
            assert_eq!(
                response.headers.get("location").map(String::as_str),
                Some(
                    "https://auth.context.example/oauth2/authorize?response_type=code&state=opaque"
                )
            );
            assert!(response.body.is_empty());
        }

        #[test]
        fn api_gateway_v2_raw_query_string_is_retained_without_normalization() {
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/auth/login",
                "rawQueryString": "state=a%2Bb%3D&scope=openid%20external.read",
                "queryStringParameters": {
                    "state": "a+b=",
                    "scope": "openid external.read"
                },
                "requestContext": { "http": { "method": "GET" } }
            }))
            .expect("API Gateway v2 login request should deserialize");

            assert_eq!(
                request.raw_query_string.as_deref(),
                Some("state=a%2Bb%3D&scope=openid%20external.read")
            );
        }

        #[test]
        fn api_key_fixture_reserved_routes_fail_closed_before_body_parsing() {
            let fixture = api_key_auth_fixture();
            for case in fixture_cases(&fixture, "route_cases") {
                let route = auth::classify_route(
                    Some(fixture_str(case, "path")),
                    case.get("method").and_then(Value::as_str),
                );
                if matches!(route, RequestRoute::OAuth | RequestRoute::Legacy) {
                    continue;
                }

                let response = reserved_route_disabled_response(route)
                    .expect("every reserved route must return before parsing a malformed body");
                assert_eq!(response.status_code, 404);
                assert_eq!(
                    serde_json::from_str::<Value>(&response.body)
                        .expect("reserved-route body should be bounded JSON"),
                    json!({ "error": { "code": "route_unavailable" } })
                );
            }
        }

        #[test]
        fn api_key_fixture_context_is_typed_versioned_and_exact_scope_checked() {
            let fixture = api_key_auth_fixture();
            let context = crate::auth::ApiKeyAuthContext::from_value(&fixture["api_key_context"])
                .expect("fixture authorizer context should satisfy v1");

            assert_eq!(context.owner_id(), "cognito:user:fixture-human");
            assert_eq!(context.key_id(), "aaaaaaaaaaaaaaaaaaaaaaaaaa");
            for tool in [
                "external_catalog",
                "external_index",
                "external_index_status",
            ] {
                let decision = crate::auth::authorize_api_key_tool(tool, Some(&context))
                    .unwrap_or_else(|error| panic!("{tool} should authorize: {error:?}"));
                assert_eq!(decision.identity.caller_id(), "cognito:user:fixture-human");
            }

            let mut wrong_version = fixture["api_key_context"].clone();
            wrong_version["auth_context_version"] = json!(2);
            assert_eq!(
                crate::auth::ApiKeyAuthContext::from_value(&wrong_version)
                    .unwrap_err()
                    .to_string(),
                "invalid API key authorizer context"
            );
            let mut delegated_management = fixture["api_key_context"].clone();
            delegated_management["scopes"] = json!("external.read keys.manage");
            assert_eq!(
                crate::auth::ApiKeyAuthContext::from_value(&delegated_management)
                    .unwrap_err()
                    .to_string(),
                "invalid API key authorizer context"
            );
        }

        #[test]
        fn api_key_fixture_management_is_human_client_and_keys_manage_only() {
            let fixture = api_key_auth_fixture();
            let issuer = fixture["discovery_config"]["issuer"]
                .as_str()
                .expect("fixture issuer");
            let config = crate::auth::AuthConfig::new(
                issuer,
                "human-client",
                ["m2m-client"],
                std::iter::empty::<&str>(),
                "urn:spur:context-service",
            );

            let decision = crate::auth::authorize_key_management(
                &config,
                Some(&fixture["human_management_claims"]),
                1_700_000_000,
            )
            .expect("human management token should authorize");
            assert_eq!(decision.identity.caller_id(), "cognito:user:fixture-human");
            assert_eq!(
                decision.identity.scheme(),
                crate::auth::AuthScheme::CognitoUser
            );
            assert_eq!(
                decision.identity.principal_kind(),
                crate::auth::PrincipalKind::Human
            );

            let mut m2m = fixture["human_management_claims"].clone();
            m2m["client_id"] = json!("m2m-client");
            m2m.as_object_mut()
                .expect("claims should be an object")
                .remove("sub");
            assert_eq!(
                crate::auth::authorize_key_management(&config, Some(&m2m), 1_700_000_000),
                Err(crate::auth::AuthFailure::HumanManagementRequired)
            );

            let mut wrong_scope = fixture["human_management_claims"].clone();
            wrong_scope["scope"] = json!("urn:spur:context-service/external.read");
            assert_eq!(
                crate::auth::authorize_key_management(&config, Some(&wrong_scope), 1_700_000_000),
                Err(crate::auth::AuthFailure::MissingScope)
            );
            assert_eq!(
                crate::auth::authorize_key_management(&config, None, 1_700_000_000),
                Err(crate::auth::AuthFailure::MissingContext)
            );
        }

        fn management_config() -> ApiKeyManagementConfig {
            ApiKeyManagementConfig::new(
                crate::auth::AuthConfig::new(
                    "https://issuer.example/pool",
                    "human-client",
                    ["m2m-client"],
                    std::iter::empty::<&str>(),
                    "urn:spur:context-service",
                ),
                crate::api_keys::KeyEnvironment::Live,
                90,
                365,
            )
            .expect("test management configuration should be valid")
        }

        fn management_request(
            method: &str,
            path: &str,
            body: Option<Value>,
            sub: &str,
        ) -> ApiGatewayRequest {
            serde_json::from_value(json!({
                "rawPath": path,
                "body": body.map(|value| value.to_string()),
                "requestContext": {
                    "authorizer": {
                        "jwt": {
                            "claims": {
                                "iss": "https://issuer.example/pool",
                                "token_use": "access",
                                "client_id": "human-client",
                                "sub": sub,
                                "exp": "2000000000",
                                "scope": "urn:spur:context-service/keys.manage"
                            }
                        }
                    },
                    "http": { "method": method }
                }
            }))
            .expect("management request should deserialize")
        }

        fn response_body(response: &ApiGatewayResponse) -> Value {
            serde_json::from_str(&response.body).expect("response body should be JSON")
        }

        #[tokio::test]
        async fn api_key_management_create_list_and_revoke_reveal_plaintext_once() {
            let now = 1_700_000_000;
            let store = crate::api_keys::FakeApiKeyStore::new();
            let create = management_request(
                "POST",
                "/auth/api-keys",
                Some(json!({
                    "name": "workstation",
                    "scopes": ["external.read", "external.index", "external.status"]
                })),
                "owner-a",
            );
            let created = handle_api_key_management(
                RequestRoute::ApiKeyCreate,
                &create,
                &store,
                &management_config(),
                now,
            )
            .await
            .expect("create response should serialize");
            let created_body = response_body(&created);
            assert_eq!(created.status_code, 201);
            assert_eq!(
                created.headers.get("cache-control").map(String::as_str),
                Some("no-store")
            );
            assert_eq!(
                created.headers.get("pragma").map(String::as_str),
                Some("no-cache")
            );
            assert!(created_body["key"]
                .as_str()
                .is_some_and(|key| key.starts_with("spur_live_")));
            assert_eq!(created_body["expires_at"], now + 90 * 86_400);
            let key_id = fixture_str(&created_body, "key_id").to_owned();

            let list = management_request("GET", "/auth/api-keys", None, "owner-a");
            let listed = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &list,
                &store,
                &management_config(),
                now,
            )
            .await
            .expect("list response should serialize");
            let listed_body = response_body(&listed);
            assert_eq!(listed.status_code, 200);
            assert!(!listed.headers.contains_key("cache-control"));
            assert!(!listed.headers.contains_key("pragma"));
            assert_eq!(listed_body["keys"][0]["key_id"], key_id);
            assert!(listed.body.find("spur_live_").is_none());
            assert!(listed.body.find("secret_hash").is_none());
            assert!(listed.body.find("cognito:user:owner-a").is_none());

            let revoke_path = format!("/auth/api-keys/{key_id}");
            let revoke = management_request("DELETE", &revoke_path, None, "owner-a");
            for _ in 0..2 {
                let revoked = handle_api_key_management(
                    RequestRoute::ApiKeyRevoke,
                    &revoke,
                    &store,
                    &management_config(),
                    now + 1,
                )
                .await
                .expect("revoke response should serialize");
                assert_eq!(revoked.status_code, 200);
                assert_eq!(response_body(&revoked)["status"], "revoked");
                assert!(revoked.body.find("spur_live_").is_none());
            }
        }

        #[tokio::test]
        async fn api_key_management_enforces_owner_scope_expiry_and_active_cap() {
            let now = 1_700_000_000;
            let store = crate::api_keys::FakeApiKeyStore::new();
            let config = management_config();
            for index in 0..10 {
                let request = management_request(
                    "POST",
                    "/auth/api-keys",
                    Some(json!({
                        "name": format!("key-{index}"),
                        "scopes": ["external.read"],
                        "expires_at": now + 365 * 86_400
                    })),
                    "owner-a",
                );
                let response = handle_api_key_management(
                    RequestRoute::ApiKeyCreate,
                    &request,
                    &store,
                    &config,
                    now,
                )
                .await
                .expect("bounded create response should serialize");
                assert_eq!(response.status_code, 201);
            }
            let eleventh = management_request(
                "POST",
                "/auth/api-keys",
                Some(json!({ "name": "eleventh", "scopes": ["external.read"] })),
                "owner-a",
            );
            let response = handle_api_key_management(
                RequestRoute::ApiKeyCreate,
                &eleventh,
                &store,
                &config,
                now,
            )
            .await
            .expect("cap response should serialize");
            assert_eq!(response.status_code, 409);
            assert_eq!(
                response_body(&response)["error"]["code"],
                "key_limit_reached"
            );

            for body in [
                json!({ "name": "delegated", "scopes": ["keys.manage"] }),
                json!({ "name": "too-long", "scopes": ["external.read"], "expires_at": now + 365 * 86_400 + 1 }),
            ] {
                let request = management_request("POST", "/auth/api-keys", Some(body), "owner-b");
                let response = handle_api_key_management(
                    RequestRoute::ApiKeyCreate,
                    &request,
                    &store,
                    &config,
                    now,
                )
                .await
                .expect("validation response should serialize");
                assert_eq!(response.status_code, 400);
            }

            let owner_a_list = management_request("GET", "/auth/api-keys", None, "owner-a");
            let owner_b_list = management_request("GET", "/auth/api-keys", None, "owner-b");
            let owner_a = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &owner_a_list,
                &store,
                &config,
                now,
            )
            .await
            .expect("owner list should serialize");
            let owner_b = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &owner_b_list,
                &store,
                &config,
                now,
            )
            .await
            .expect("owner list should serialize");
            assert_eq!(
                response_body(&owner_a)["keys"].as_array().map(Vec::len),
                Some(10)
            );
            assert_eq!(response_body(&owner_b)["keys"], json!([]));

            let key_id = response_body(&owner_a)["keys"][0]["key_id"]
                .as_str()
                .expect("key id should be present")
                .to_owned();
            let cross_owner = management_request(
                "DELETE",
                &format!("/auth/api-keys/{key_id}"),
                None,
                "owner-b",
            );
            let response = handle_api_key_management(
                RequestRoute::ApiKeyRevoke,
                &cross_owner,
                &store,
                &config,
                now,
            )
            .await
            .expect("cross-owner response should serialize");
            assert_eq!(response.status_code, 404);
            assert_eq!(response_body(&response)["error"]["code"], "not_found");
        }

        #[tokio::test]
        async fn api_key_create_retries_only_public_id_collisions_and_reveals_the_winner() {
            let now = 1_700_000_000;
            let owner = "cognito:user:collision-owner";
            let scopes = ApiKeyScopes::parse(&["external.read"]).expect("scope should be valid");
            let collision = generate_api_key(
                KeyEnvironment::Live,
                owner,
                "collision",
                scopes.clone(),
                now,
                now + 3_600,
            )
            .expect("collision key should generate");
            let collision_plaintext = collision.plaintext.expose_secret().to_owned();
            let winner = generate_api_key(
                KeyEnvironment::Live,
                owner,
                "collision",
                scopes,
                now,
                now + 3_600,
            )
            .expect("winner key should generate");
            let winner_plaintext = winner.plaintext.expose_secret().to_owned();
            let store = crate::api_keys::FakeApiKeyStore::new();
            store
                .create_key(CreateKeyRecord::new(collision.record.clone()))
                .await
                .expect("collision record should already exist");
            let mut generated = std::collections::VecDeque::from([collision, winner]);
            let request = management_request(
                "POST",
                "/auth/api-keys",
                Some(json!({ "name": "collision", "scopes": ["external.read"] })),
                "collision-owner",
            );

            let response = create_api_key_with_generator(
                &request,
                &store,
                &management_config(),
                owner,
                now,
                |_, _, _, _, _, _| {
                    generated
                        .pop_front()
                        .ok_or(crate::api_keys::ApiKeyError::GenerationUnavailable)
                },
            )
            .await
            .expect("duplicate followed by success should return a response");

            assert_eq!(response.status_code, 201);
            assert_eq!(response_body(&response)["key"], winner_plaintext);
            assert!(!response.body.contains(&collision_plaintext));
            assert!(generated.is_empty());
        }

        #[tokio::test]
        async fn api_key_create_bounds_collision_retries_and_does_not_retry_owner_limit() {
            let now = 1_700_000_000;
            let owner = "cognito:user:retry-owner";
            let scopes = ApiKeyScopes::parse(&["external.read"]).expect("scope should be valid");
            let store = crate::api_keys::FakeApiKeyStore::new();
            let mut collisions = std::collections::VecDeque::new();
            for index in 0..3 {
                let generated = generate_api_key(
                    KeyEnvironment::Live,
                    owner,
                    &format!("collision-{index}"),
                    scopes.clone(),
                    now,
                    now + 3_600,
                )
                .expect("collision key should generate");
                store
                    .create_key(CreateKeyRecord::new(generated.record.clone()))
                    .await
                    .expect("collision record should already exist");
                collisions.push_back(generated);
            }
            let request = management_request(
                "POST",
                "/auth/api-keys",
                Some(json!({ "name": "collision", "scopes": ["external.read"] })),
                "retry-owner",
            );
            let response = create_api_key_with_generator(
                &request,
                &store,
                &management_config(),
                owner,
                now,
                |_, _, _, _, _, _| {
                    collisions
                        .pop_front()
                        .ok_or(crate::api_keys::ApiKeyError::GenerationUnavailable)
                },
            )
            .await
            .expect("retry exhaustion should return a bounded response");
            assert_eq!(response.status_code, 503);
            assert_eq!(
                response_body(&response)["error"]["code"],
                "key_store_unavailable"
            );
            assert!(collisions.is_empty());

            let capped_owner = "cognito:user:capped-owner";
            for index in 0..10 {
                let generated = generate_api_key(
                    KeyEnvironment::Live,
                    capped_owner,
                    &format!("cap-{index}"),
                    ApiKeyScopes::parse(&["external.read"]).expect("scope should be valid"),
                    now,
                    now + 3_600,
                )
                .expect("cap key should generate");
                store
                    .create_key(CreateKeyRecord::new(generated.record))
                    .await
                    .expect("cap record should persist");
            }
            let mut attempts = 0;
            let capped_request = management_request(
                "POST",
                "/auth/api-keys",
                Some(json!({ "name": "capped", "scopes": ["external.read"] })),
                "capped-owner",
            );
            let response = create_api_key_with_generator(
                &capped_request,
                &store,
                &management_config(),
                capped_owner,
                now,
                |environment, owner, name, scopes, created_at, expires_at| {
                    attempts += 1;
                    generate_api_key(environment, owner, name, scopes, created_at, expires_at)
                },
            )
            .await
            .expect("owner cap should return a bounded response");
            assert_eq!(response.status_code, 409);
            assert_eq!(attempts, 1);
        }

        #[tokio::test]
        async fn api_key_list_accepts_bounded_query_pagination_and_reaches_history() {
            let now = 1_700_000_000;
            let owner = "cognito:user:history-owner";
            let store = crate::api_keys::FakeApiKeyStore::new();
            for index in 0..101_u64 {
                let generated = generate_api_key(
                    KeyEnvironment::Live,
                    owner,
                    &format!("history-{index}"),
                    ApiKeyScopes::parse(&["external.read"]).expect("scope should be valid"),
                    now + index,
                    now + 10_000,
                )
                .expect("historical key should generate");
                let key_id = generated.public_id.clone();
                store
                    .create_key(CreateKeyRecord::new(generated.record))
                    .await
                    .expect("historical key should persist");
                assert_eq!(
                    store
                        .revoke_key(owner, &key_id, now + 1_000)
                        .await
                        .expect("historical key should revoke"),
                    RevokeResult::Revoked
                );
            }

            let first = management_request(
                "GET",
                "/auth/api-keys",
                Some(json!({ "cursor": "not-a-cursor", "limit": 1 })),
                "history-owner",
            );
            let first = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &first,
                &store,
                &management_config(),
                now,
            )
            .await
            .expect("first history page should serialize");
            let first_body = response_body(&first);
            assert_eq!(first_body["keys"].as_array().map(Vec::len), Some(100));
            let cursor = fixture_str(&first_body, "next_cursor");

            let second = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/auth/api-keys",
                "queryStringParameters": { "cursor": cursor, "limit": "100" },
                "requestContext": {
                    "authorizer": { "jwt": { "claims": {
                        "iss": "https://issuer.example/pool",
                        "token_use": "access",
                        "client_id": "human-client",
                        "sub": "history-owner",
                        "exp": "2000000000",
                        "scope": "urn:spur:context-service/keys.manage"
                    }}},
                    "http": { "method": "GET" }
                }
            }))
            .expect("paginated request should deserialize");
            let second = handle_api_key_management(
                RequestRoute::ApiKeyList,
                &second,
                &store,
                &management_config(),
                now,
            )
            .await
            .expect("second history page should serialize");
            assert_eq!(
                response_body(&second)["keys"].as_array().map(Vec::len),
                Some(1)
            );

            for parameters in [
                json!({ "limit": "0" }),
                json!({ "limit": "101" }),
                json!({ "limit": "many" }),
                json!({ "limit": "+1" }),
                json!({ "limit": "01" }),
                json!({ "cursor": "not-a-cursor" }),
            ] {
                let malformed = serde_json::from_value::<ApiGatewayRequest>(json!({
                    "rawPath": "/auth/api-keys",
                    "queryStringParameters": parameters,
                    "requestContext": {
                        "authorizer": { "jwt": { "claims": {
                            "iss": "https://issuer.example/pool",
                            "token_use": "access",
                            "client_id": "human-client",
                            "sub": "history-owner",
                            "exp": "2000000000",
                            "scope": "urn:spur:context-service/keys.manage"
                        }}},
                        "http": { "method": "GET" }
                    }
                }))
                .expect("malformed query request should deserialize");
                let response = handle_api_key_management(
                    RequestRoute::ApiKeyList,
                    &malformed,
                    &store,
                    &management_config(),
                    now,
                )
                .await
                .expect("malformed query should return a bounded response");
                assert_eq!(response.status_code, 400);
                assert_eq!(response_body(&response)["error"]["code"], "invalid_request");
            }
        }

        #[test]
        fn api_key_mcp_trusts_only_v1_lambda_context_and_rechecks_body_scope() {
            let fixture = api_key_auth_fixture();
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/api-key",
                "requestContext": {
                    "authorizer": {
                        "lambda": fixture["api_key_context"].clone(),
                        "principalId": "fallback-principal",
                        "iam": {
                            "accountId": "123456789012",
                            "userId": "AROAFALLBACK:session"
                        }
                    },
                    "http": { "method": "POST", "sourceIp": "203.0.113.24" }
                }
            }))
            .expect("API-key request should deserialize");

            let decision = authorize_api_key_request(&request, "external_index")
                .expect("context with external.index should authorize");
            assert_eq!(decision.identity.caller_id(), "cognito:user:fixture-human");

            let mut read_only = fixture["api_key_context"].clone();
            read_only["scopes"] = json!("external.read");
            let read_only_request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/api-key",
                "requestContext": {
                    "authorizer": { "lambda": read_only },
                    "http": { "method": "POST" }
                }
            }))
            .expect("read-only API-key request should deserialize");
            assert_eq!(
                authorize_api_key_request(&read_only_request, "external_index"),
                Err(crate::auth::AuthFailure::MissingScope)
            );

            let fallback_only = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/api-key",
                "requestContext": {
                    "authorizer": {
                        "principalId": "fallback-principal",
                        "iam": { "accountId": "123456789012", "userId": "AROAFALLBACK:session" },
                        "jwt": { "claims": fixture["human_management_claims"].clone() }
                    },
                    "http": { "method": "POST", "sourceIp": "203.0.113.24" }
                }
            }))
            .expect("wrong-context API-key request should deserialize");
            assert_eq!(
                authorize_api_key_request(&fallback_only, "external_catalog"),
                Err(crate::auth::AuthFailure::MissingContext)
            );
        }

        #[test]
        fn api_key_context_on_any_other_route_fails_without_legacy_downgrade() {
            let fixture = api_key_auth_fixture();
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp",
                "requestContext": {
                    "authorizer": {
                        "lambda": fixture["api_key_context"].clone(),
                        "principalId": "fallback-principal"
                    },
                    "http": { "method": "POST", "sourceIp": "203.0.113.24" }
                }
            }))
            .expect("wrong-route context should deserialize");

            assert_eq!(
                reject_api_key_auth_on_wrong_route(&request),
                Err(crate::auth::AuthFailure::WrongRoute)
            );
        }

        #[test]
        fn poc_external_index_fixture_parses_through_oauth_request_contract() {
            let request = ApiGatewayRequest {
                body: Some(poc_external_index_fixture().to_owned()),
                is_base64_encoded: false,
                path: None,
                raw_path: Some("/mcp/oauth".to_owned()),
                raw_query_string: None,
                query_string_parameters: None,
                request_context: Some(ApiGatewayRequestContext {
                    authorizer: None,
                    http: Some(ApiGatewayHttp {
                        method: Some("POST".to_owned()),
                        source_ip: None,
                    }),
                    identity: None,
                }),
            };

            assert!(is_oauth_route(&request));
            let parsed = parse_tool_request(&request)
                .expect("the exact committed POC body should satisfy the OAuth request contract");

            assert_eq!(parsed.tool, "external_index");
            assert_eq!(
                parsed.args,
                json!({
                    "package": "validation-only-fixture",
                    "revision": "offline",
                    "source_url": "https://validation-only.invalid/spur-context-poc.tar.gz",
                    "source_kind": "tarball",
                    "force": false,
                })
            );
        }

        struct EnvVarRestore {
            name: &'static str,
            previous: Option<std::ffi::OsString>,
        }

        impl EnvVarRestore {
            fn set(name: &'static str, value: &str) -> Self {
                let previous = std::env::var_os(name);
                std::env::set_var(name, value);
                Self { name, previous }
            }
        }

        impl Drop for EnvVarRestore {
            fn drop(&mut self) {
                match &self.previous {
                    Some(value) => std::env::set_var(self.name, value),
                    None => std::env::remove_var(self.name),
                }
            }
        }

        #[test]
        fn eventbridge_schedule_routes_to_queue_drainer() {
            let event = json!({
                "source": "aws.events",
                "detail-type": "Scheduled Event",
                "detail": {
                    "operation": "drain_queued_jobs"
                }
            });

            assert!(is_scheduled_drainer_event(&event));
        }

        #[test]
        fn hybrid_auth_fixture_covers_scope_identity_denial_and_route_contracts() {
            let fixture = hybrid_auth_fixture();
            let config = crate::auth::AuthConfig::new(
                "https://issuer.example/pool",
                "human-client",
                ["m2m-client", "rotating-m2m-client"],
                ["blocked-client"],
                "urn:spur:context-service",
            );

            let fixture_policy = fixture_cases(&fixture, "scope_cases")
                .iter()
                .map(|case| {
                    (
                        fixture_str(case, "tool"),
                        fixture_str(case, "required_scope"),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            assert_eq!(fixture_policy, crate::auth::external_tool_scopes());
            let all_scopes = fixture_policy
                .values()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();

            for (tool, required_scope) in fixture_policy {
                for candidate_scope in &all_scopes {
                    let claims = json!({
                        "iss": "https://issuer.example/pool",
                        "token_use": "access",
                        "client_id": "human-client",
                        "sub": "fixture-human",
                        "exp": 2_000_000_000_u64,
                        "scope": candidate_scope,
                    });
                    let result = crate::auth::authorize_oauth_tool(
                        &config,
                        tool,
                        Some(&claims),
                        1_700_000_000,
                    );

                    if *candidate_scope == required_scope {
                        let decision = result.unwrap_or_else(|error| {
                            panic!("{tool} should authorize {candidate_scope}: {error:?}")
                        });
                        assert_eq!(decision.identity.caller_id(), "cognito:user:fixture-human");
                    } else {
                        assert_eq!(
                            result,
                            Err(crate::auth::AuthFailure::MissingScope),
                            "{tool} must reject nonmatching scope {candidate_scope}"
                        );
                    }
                }
            }

            for case in fixture_cases(&fixture, "identity_cases") {
                let request = serde_json::from_value::<ApiGatewayRequest>(case["event"].clone())
                    .unwrap_or_else(|error| {
                        panic!("{} should deserialize: {error}", fixture_str(case, "name"))
                    });
                let caller_id = match fixture_str(case, "scheme") {
                    "oauth" => authorize_oauth_request(
                        &request,
                        fixture_str(case, "tool"),
                        &config,
                        1_700_000_000,
                    )
                    .unwrap_or_else(|error| {
                        panic!("{} should authorize: {error:?}", fixture_str(case, "name"))
                    })
                    .identity
                    .caller_id()
                    .to_owned(),
                    "legacy" => authenticated_caller_id(
                        &request,
                        case["allow_anonymous"].as_bool().unwrap_or(false),
                    )
                    .unwrap_or_else(|error| {
                        panic!("{} should authenticate: {error}", fixture_str(case, "name"))
                    }),
                    scheme => panic!("unsupported fixture auth scheme {scheme:?}"),
                };
                assert_eq!(caller_id, fixture_str(case, "expected_caller"));
            }

            for case in fixture_cases(&fixture, "denial_cases") {
                let request = serde_json::from_value::<ApiGatewayRequest>(case["event"].clone())
                    .unwrap_or_else(|error| {
                        panic!("{} should deserialize: {error}", fixture_str(case, "name"))
                    });
                assert!(is_oauth_route(&request));
                let failure = authorize_oauth_request(
                    &request,
                    fixture_str(case, "tool"),
                    &config,
                    1_700_000_000,
                )
                .expect_err("denial fixture should fail closed");
                let reason = failure.reason();
                let response = authorization_error_response(failure)
                    .expect("bounded authorization response should serialize");
                let expected_status = u16::try_from(
                    case["expected_status"]
                        .as_u64()
                        .expect("expected_status should be numeric"),
                )
                .expect("expected_status should fit in an HTTP status code");

                assert_eq!(
                    response.status_code,
                    expected_status,
                    "{}",
                    fixture_str(case, "name")
                );
                assert_eq!(reason, fixture_str(case, "expected_reason"));
                assert!(response.body.contains(reason));
                assert!(!response.body.contains("fallback-principal"));
                assert!(!response.body.contains("AROAFALLBACK"));
                assert!(!response.body.contains("203.0.113.24"));
            }

            for case in fixture_cases(&fixture, "route_cases") {
                let request = serde_json::from_value::<ApiGatewayRequest>(case["event"].clone())
                    .unwrap_or_else(|error| {
                        panic!("{} should deserialize: {error}", fixture_str(case, "name"))
                    });
                assert_eq!(
                    is_oauth_route(&request),
                    case["is_oauth"].as_bool().expect("is_oauth should be bool"),
                    "{}",
                    fixture_str(case, "name")
                );
            }

            assert!(is_scheduled_drainer_event(
                &fixture["scheduled_drainer_event"]
            ));
            assert!(!is_scheduled_drainer_event(&fixture["oauth_http_event"]));
        }

        #[tokio::test]
        async fn scheduled_drainer_fixture_bypasses_http_deserialization_and_auth() {
            let event = LambdaEvent::new(
                hybrid_auth_fixture()["scheduled_drainer_event"].clone(),
                lambda_runtime::Context::default(),
            );
            let response = handle_event_with_drainer(event, async {
                Ok(drainer::DrainSummary {
                    dispatched: 2,
                    skipped: 1,
                    failed: 0,
                    repaired: 0,
                })
            })
            .await
            .expect("scheduled event should run the injected drainer before HTTP parsing");

            assert_eq!(
                response,
                json!({
                    "operation": "drain_queued_jobs",
                    "dispatched": 2,
                    "skipped": 1,
                    "failed": 0,
                    "repaired": 0,
                })
            );
        }

        #[tokio::test]
        async fn public_knowledge_handler_preserves_scheduled_drainer_path() {
            let event = LambdaEvent::new(
                json!({
                    "source": "aws.events",
                    "detail-type": "Scheduled Event",
                    "detail": {
                        "operation": "drain_queued_jobs"
                    }
                }),
                lambda_runtime::Context::default(),
            );
            let response = with_test_drainer_summary(
                drainer::DrainSummary {
                    dispatched: 3,
                    skipped: 2,
                    failed: 1,
                    repaired: 4,
                },
                handler(event),
            )
            .await
            .expect("public Knowledge handler should bypass HTTP parsing and auth");

            assert_eq!(
                response,
                json!({
                    "operation": "drain_queued_jobs",
                    "dispatched": 3,
                    "skipped": 2,
                    "failed": 1,
                    "repaired": 4,
                })
            );
        }

        #[test]
        fn jwt_fixture_on_wrong_route_is_rejected_without_identity_downgrade() {
            let request = serde_json::from_value::<ApiGatewayRequest>(
                hybrid_auth_fixture()["wrong_route_jwt_event"].clone(),
            )
            .expect("wrong-route JWT fixture should deserialize");

            let failure = reject_jwt_auth_on_wrong_route(&request)
                .expect_err("JWT context on the legacy route must fail closed");
            let reason = failure.reason();
            let response = authorization_error_response(failure)
                .expect("bounded wrong-route response should serialize");

            assert_eq!(response.status_code, 401);
            assert_eq!(reason, "wrong_route");
            assert!(!response.body.contains("fallback-principal"));
            assert!(!response.body.contains("AROAFALLBACK"));
            assert!(!response.body.contains("203.0.113.24"));
            assert!(!response.body.contains("anonymous-internal"));
        }

        #[tokio::test]
        async fn api_gateway_event_keeps_proxy_response_shape() {
            let event = LambdaEvent::new(
                json!({
                    "body": null,
                    "isBase64Encoded": false
                }),
                lambda_runtime::Context::default(),
            );

            let response = handler(event)
                .await
                .expect("invalid tool input should return an API Gateway error response");

            assert_eq!(response["statusCode"], 200);
            assert!(response["body"].is_string());
            assert_eq!(response["isBase64Encoded"], false);
        }

        fn request_from_context(request_context: Value) -> ApiGatewayRequest {
            serde_json::from_value(json!({
                "body": "{}",
                "requestContext": request_context
            }))
            .expect("API Gateway request should deserialize")
        }

        #[test]
        fn caller_id_prefers_http_api_v2_jwt_subject() {
            let request = request_from_context(json!({
                "authorizer": {
                    "principalId": "rest-principal",
                    "jwt": {
                        "claims": {
                            "sub": "jwt-subject"
                        }
                    }
                },
                "http": {
                    "sourceIp": "203.0.113.24"
                },
                "identity": {
                    "userArn": "arn:aws:iam::123456789012:user/rest",
                    "sourceIp": "198.51.100.10"
                }
            }));

            assert_eq!(caller_id(&request), "jwt-subject");
        }

        #[test]
        fn caller_id_uses_http_api_v2_source_ip_before_rest_identity() {
            let request = request_from_context(json!({
                "http": {
                    "sourceIp": "203.0.113.24"
                },
                "identity": {
                    "userArn": "arn:aws:iam::123456789012:user/rest",
                    "sourceIp": "198.51.100.10"
                }
            }));

            assert_eq!(caller_id(&request), "203.0.113.24");
        }

        #[test]
        fn caller_id_keeps_rest_api_v1_principal_fallback() {
            let request = request_from_context(json!({
                "authorizer": {
                    "principalId": "rest-principal"
                },
                "identity": {
                    "userArn": "arn:aws:iam::123456789012:user/rest",
                    "sourceIp": "198.51.100.10"
                }
            }));

            assert_eq!(caller_id(&request), "rest-principal");
        }

        #[test]
        fn authenticated_caller_id_accepts_http_api_iam_user_arn() {
            let request = request_from_context(json!({
                "authorizer": {
                    "iam": {
                        "userArn": "arn:aws:iam::123456789012:role/context-indexer",
                        "callerId": "AROATEST:session"
                    }
                },
                "http": {
                    "sourceIp": "203.0.113.24"
                }
            }));

            assert_eq!(
                authenticated_caller_id(&request, false).expect("IAM caller should authenticate"),
                "arn:aws:iam::123456789012:role/context-indexer"
            );
        }

        #[test]
        fn authenticated_caller_id_uses_stable_iam_principal_without_session_name() {
            let request = request_from_context(json!({
                "authorizer": {
                    "iam": {
                        "accountId": "123456789012",
                        "userId": "AROASTABLE:untrusted-session-name",
                        "userArn": "arn:aws:sts::123456789012:assumed-role/context-indexer/untrusted-session-name"
                    }
                }
            }));

            assert_eq!(
                authenticated_caller_id(&request, false).expect("IAM caller should authenticate"),
                "iam:123456789012:AROASTABLE"
            );
        }

        #[test]
        fn authenticated_caller_id_rejects_source_ip_only_request() {
            let request = request_from_context(json!({
                "http": {
                    "sourceIp": "203.0.113.24"
                }
            }));

            let error = authenticated_caller_id(&request, false).unwrap_err();

            assert!(error.to_string().contains("authenticated caller"));
        }

        #[test]
        fn authenticated_caller_id_falls_back_to_anonymous_when_allowed() {
            // Public (NONE auth) request: no authorizer/identity caller present.
            let request = request_from_context(json!({
                "http": {
                    "sourceIp": "203.0.113.24"
                }
            }));

            assert_eq!(
                authenticated_caller_id(&request, true)
                    .expect("anonymous fallback should authenticate when allowed"),
                ANONYMOUS_CALLER_ID
            );
        }

        #[test]
        fn authenticated_caller_id_prefers_real_caller_over_anonymous_fallback() {
            // Even with anonymous allowed, a real authenticated caller wins.
            let request = request_from_context(json!({
                "identity": {
                    "userArn": "arn:aws:iam::123456789012:user/real"
                }
            }));

            assert_eq!(
                authenticated_caller_id(&request, true).expect("real caller should authenticate"),
                "arn:aws:iam::123456789012:user/real"
            );
        }

        #[test]
        fn oauth_route_requires_the_exact_post_path() {
            let oauth = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/oauth",
                "requestContext": { "http": { "method": "POST" } }
            }))
            .expect("OAuth request should deserialize");
            let wrong_method = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/oauth",
                "requestContext": { "http": { "method": "GET" } }
            }))
            .expect("non-POST request should deserialize");

            assert!(is_oauth_route(&oauth));
            assert!(!is_oauth_route(&wrong_method));
        }

        #[test]
        fn oauth_route_cannot_be_moved_by_an_environment_override() {
            let _path = EnvVarRestore::set("SPUR_COGNITO_OAUTH_PATH", "/different-path");
            let oauth = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/oauth",
                "requestContext": { "http": { "method": "POST" } }
            }))
            .expect("OAuth request should deserialize");

            assert!(is_oauth_route(&oauth));
        }

        #[test]
        fn oauth_api_gateway_string_claims_authorize_human_identity() {
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/oauth",
                "requestContext": {
                    "authorizer": {
                        "jwt": {
                            "claims": {
                                "iss": "https://issuer.example/pool",
                                "token_use": "access",
                                "client_id": "human-client",
                                "sub": "human-subject",
                                "exp": "2000000000",
                                "scope": "urn:spur:context-service/external.read"
                            }
                        }
                    },
                    "http": { "method": "POST" }
                }
            }))
            .expect("API Gateway JWT claims should deserialize as strings");
            let config = crate::auth::AuthConfig::new(
                "https://issuer.example/pool",
                "human-client",
                ["m2m-client"],
                std::iter::empty::<&str>(),
                "urn:spur:context-service",
            );

            let decision =
                authorize_oauth_request(&request, "external_catalog", &config, 1_700_000_000)
                    .expect("a valid API Gateway JWT claim map should authorize");

            assert_eq!(decision.identity.caller_id(), "cognito:user:human-subject");
        }

        #[test]
        fn malformed_oauth_jwt_never_falls_back_to_iam_or_principal_id() {
            let request = serde_json::from_value::<ApiGatewayRequest>(json!({
                "rawPath": "/mcp/oauth",
                "requestContext": {
                    "authorizer": {
                        "principalId": "legacy-principal",
                        "iam": {
                            "accountId": "123456789012",
                            "userId": "AROATEST:session"
                        },
                        "jwt": {
                            "claims": {
                                "iss": "unexpected-issuer",
                                "token_use": "access",
                                "client_id": "human-client",
                                "sub": "human-subject",
                                "exp": 2000000000,
                                "scope": "urn:spur:context-service/external.read"
                            }
                        }
                    },
                    "http": { "method": "POST", "sourceIp": "203.0.113.24" }
                }
            }))
            .expect("OAuth request should deserialize");
            let config = crate::auth::AuthConfig::new(
                "https://issuer.example/pool",
                "human-client",
                ["m2m-client"],
                std::iter::empty::<&str>(),
                "urn:spur:context-service",
            );

            assert_eq!(
                authorize_oauth_request(&request, "external_catalog", &config, 1_700_000_000),
                Err(crate::auth::AuthFailure::WrongIssuer)
            );
        }

        #[test]
        fn oauth_errors_return_bounded_401_or_403_bodies() {
            let unauthorized = authorization_error_response(crate::auth::AuthFailure::WrongIssuer)
                .expect("authorization response should serialize");
            let forbidden = authorization_error_response(crate::auth::AuthFailure::MissingScope)
                .expect("authorization response should serialize");

            assert_eq!(unauthorized.status_code, 401);
            assert_eq!(forbidden.status_code, 403);
            assert!(unauthorized.body.contains("wrong_issuer"));
            assert!(!unauthorized.body.contains("token"));
        }

        #[test]
        fn remote_catalog_dsn_reinitializes_when_etag_changes() {
            let s3_dsn = "s3://example-context/catalog/catalog.ducklake";

            assert!(should_initialize_catalog(
                None,
                None,
                s3_dsn,
                Some("etag-a")
            ));
            assert!(!should_initialize_catalog(
                Some(s3_dsn),
                Some("etag-a"),
                s3_dsn,
                Some("etag-a")
            ));
            assert!(should_initialize_catalog(
                Some(s3_dsn),
                Some("etag-a"),
                s3_dsn,
                Some("etag-b")
            ));
            assert!(should_initialize_catalog(
                Some(s3_dsn),
                Some("etag-a"),
                s3_dsn,
                None
            ));

            assert!(should_initialize_catalog(
                None,
                None,
                "sqlite:/tmp/catalog.sqlite",
                None
            ));
            assert!(!should_initialize_catalog(
                Some("sqlite:/tmp/catalog.sqlite"),
                None,
                "sqlite:/tmp/catalog.sqlite",
                None
            ));
        }

        #[test]
        fn pointer_cache_key_changes_when_live_pointer_switches_generation() {
            let pointer_uri = "s3://example-context/gold/catalog-snapshot/current.json";
            let first = catalog::FrozenSnapshotManifest::published(
            10,
            "s3://example-context/gold/catalog-snapshot/generations/00000000000000000010/spur_context.ducklake".to_owned(),
            "s3://example-context/gold/data/".to_owned(),
            "sha10".to_owned(),
            10,
        );
            let second = catalog::FrozenSnapshotManifest::published(
            11,
            "s3://example-context/gold/catalog-snapshot/generations/00000000000000000011/spur_context.ducklake".to_owned(),
            "s3://example-context/gold/data/".to_owned(),
            "sha11".to_owned(),
            11,
        );

            let first_key = snapshot_pointer_cache_key(pointer_uri, Some("etag-a"), &first);
            let second_key = snapshot_pointer_cache_key(pointer_uri, Some("etag-b"), &second);

            assert_ne!(first_key, second_key);
            assert!(should_initialize_catalog(
                Some(&first_key),
                Some("etag-a"),
                &second_key,
                Some("etag-b")
            ));
        }

        #[test]
        fn pointer_cache_key_supports_rollback_to_previous_generation() {
            let pointer_uri = "s3://example-context/gold/catalog-snapshot/current.json";
            let current = catalog::FrozenSnapshotManifest::published(
            11,
            "s3://example-context/gold/catalog-snapshot/generations/00000000000000000011/spur_context.ducklake".to_owned(),
            "s3://example-context/gold/data/".to_owned(),
            "sha11".to_owned(),
            11,
        );
            let rollback = catalog::FrozenSnapshotManifest::published(
            10,
            "s3://example-context/gold/catalog-snapshot/generations/00000000000000000010/spur_context.ducklake".to_owned(),
            "s3://example-context/gold/data/".to_owned(),
            "sha10".to_owned(),
            10,
        );

            let current_key =
                snapshot_pointer_cache_key(pointer_uri, Some("etag-current"), &current);
            let rollback_key =
                snapshot_pointer_cache_key(pointer_uri, Some("etag-rollback"), &rollback);

            assert_ne!(current_key, rollback_key);
            assert!(rollback_key.contains("generation=10"));
            assert!(rollback_key.contains(&rollback.snapshot_uri));
        }

        #[tokio::test]
        async fn corrupt_cached_snapshot_is_not_sticky_across_repair_attempts() {
            let local_path = env::temp_dir().join(format!(
                "spur-context-service-corrupt-snapshot-{}",
                uuid::Uuid::new_v4()
            ));
            fs::write(&local_path, b"corrupt cache")
                .expect("corrupt cache fixture should be written");

            let valid_snapshot = b"valid frozen catalog snapshot";
            let mut hasher = Sha256::new();
            hasher.update(valid_snapshot);
            let expected_sha256 = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();

            let first_error = ensure_local_snapshot(&local_path, &expected_sha256, || async {
                tokio::fs::write(&local_path, b"corrupt replacement")
                    .await
                    .map_err(Error::from)
            })
            .await
            .expect_err("a corrupt replacement should fail integrity verification");

            assert!(first_error.to_string().contains("sha256 mismatch"));
            assert!(
                !local_path.exists(),
                "failed integrity verification must not leave a poisoned cache entry"
            );

            ensure_local_snapshot(&local_path, &expected_sha256, || async {
                tokio::fs::write(&local_path, valid_snapshot)
                    .await
                    .map_err(Error::from)
            })
            .await
            .expect("the next repair attempt should install a valid snapshot");

            assert_eq!(
                fs::read(&local_path).expect("repaired snapshot should be readable"),
                valid_snapshot
            );
            fs::remove_file(&local_path).expect("snapshot fixture should be removed");
        }
    }
}
