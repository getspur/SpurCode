//! Local helper configuration without executable discovery or native initialization.
use crate::{ClientPolicy, ProtocolError, ProtocolLimits};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;

/// Numeric admission and deadline policy, independent of helper discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPolicy {
    pub limits: ProtocolLimits,
    pub artifact_budget: u64,
    pub queue_capacity: usize,
    pub max_restarts: usize,
    pub startup_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub shutdown_poll_interval_ms: u64,
}

impl Default for LocalPolicy {
    fn default() -> Self {
        // Root-reviewed operating choices and captured revision identities:
        // docs/research/artifacts/2026-11-05-embedding-sol/M2-pre/policy.json.
        // These are configurable admission budgets, not future latency/RSS guarantees.
        Self {
            limits: ProtocolLimits {
                max_frame_bytes: 33_554_432,
                max_texts: 64,
                max_text_bytes: 65_536,
                max_total_text_bytes: 4_194_304,
                max_dimensions: 768,
                max_identity_bytes: 64,
                max_pending: 1,
            },
            artifact_budget: 644_080_771,
            queue_capacity: 1,
            max_restarts: 0,
            startup_timeout_ms: 120_000,
            // One request deadline covers both Hello and first inference.
            request_timeout_ms: 240_000,
            shutdown_timeout_ms: 30_000,
            shutdown_poll_interval_ms: 20,
        }
    }
}

/// Existing complete JSON override format. Every field remains required.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperConfig {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub artifact_budget: u64,
    pub limits: ProtocolLimits,
    pub queue_capacity: usize,
    pub max_restarts: usize,
    pub startup_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub shutdown_poll_interval_ms: u64,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("invalid SPUR_EMBEDDING_HELPER_CONFIG: {0}")]
    Environment(#[from] std::env::VarError),
    #[error("invalid embedding helper configuration: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid embedding protocol limits: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("invalid embedding helper policy: {0}")]
    Invalid(&'static str),
}

impl HelperConfig {
    /// Parse and validate an explicit override without checking or launching its path.
    pub fn from_json(raw: &str) -> Result<Self, PolicyError> {
        let config: Self = serde_json::from_str(raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.executable.as_os_str().is_empty() {
            return Err(PolicyError::Invalid("empty executable"));
        }
        self.policy().validate()
    }

    pub fn policy(&self) -> LocalPolicy {
        LocalPolicy {
            limits: self.limits.clone(),
            artifact_budget: self.artifact_budget,
            queue_capacity: self.queue_capacity,
            max_restarts: self.max_restarts,
            startup_timeout_ms: self.startup_timeout_ms,
            request_timeout_ms: self.request_timeout_ms,
            shutdown_timeout_ms: self.shutdown_timeout_ms,
            shutdown_poll_interval_ms: self.shutdown_poll_interval_ms,
        }
    }
}

impl LocalPolicy {
    /// An absent override selects defaults; a supplied invalid override is an error.
    /// Reading policy never discovers a helper or touches model/runtime caches.
    pub fn from_env() -> Result<Self, PolicyError> {
        let policy = match std::env::var("SPUR_EMBEDDING_HELPER_CONFIG") {
            Ok(raw) => HelperConfig::from_json(&raw)?.policy(),
            Err(std::env::VarError::NotPresent) => Self::default(),
            Err(error) => return Err(error.into()),
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        self.limits.validate()?;
        if self.artifact_budget == 0 {
            return Err(PolicyError::Invalid("zero artifact budget"));
        }
        if self.queue_capacity == 0 || self.queue_capacity.checked_add(1).is_none() {
            return Err(PolicyError::Invalid("zero or overflowing queue capacity"));
        }
        let now = Instant::now();
        for ms in [
            self.startup_timeout_ms,
            self.request_timeout_ms,
            self.shutdown_timeout_ms,
            self.shutdown_poll_interval_ms,
        ] {
            if ms == 0 || now.checked_add(Duration::from_millis(ms)).is_none() {
                return Err(PolicyError::Invalid("zero or overflowing deadline"));
            }
        }
        Ok(())
    }

    /// Validate before projecting into the supervisor's duration-based policy.
    pub fn client_policy(&self) -> Result<ClientPolicy, PolicyError> {
        self.validate()?;
        Ok(ClientPolicy {
            queue_capacity: self.queue_capacity,
            max_restarts: self.max_restarts,
            startup_timeout: Duration::from_millis(self.startup_timeout_ms),
            request_timeout: Duration::from_millis(self.request_timeout_ms),
            shutdown_timeout: Duration::from_millis(self.shutdown_timeout_ms),
            shutdown_poll_interval: Duration::from_millis(self.shutdown_poll_interval_ms),
        })
    }
}
