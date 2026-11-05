//! Immutable local command selection. No model or native cache is materialized here.
use crate::{
    Cancellation, CleanupOwner, ClientError, EmbeddingClient, HelperConfig, LocalPolicy,
    PolicyError, PROTOCOL_VERSION,
};
use serde::Deserialize;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct HelperCommand {
    program: OsString,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
}

impl HelperCommand {
    pub fn new(program: impl Into<OsString>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
            environment: Vec::new(),
        }
    }
    pub fn program(&self) -> &std::ffi::OsStr {
        &self.program
    }
    pub fn args(&self) -> &[OsString] {
        &self.args
    }
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }
    pub fn with_args(&self, args: Vec<OsString>) -> Self {
        Self {
            args,
            ..self.clone()
        }
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error("helper executable missing: {0:?}")]
    Missing(PathBuf),
    #[error("invalid helper path {path:?}: {reason}")]
    Path { path: PathBuf, reason: String },
    #[error("invalid helper cache: {0}")]
    Cache(&'static str),
    #[error("invalid helper metadata: {0}")]
    Metadata(String),
    #[error("helper metadata exceeds {0} bytes")]
    MetadataLimit(u32),
    #[error("helper compatibility exit: {0}")]
    Exit(String),
    #[error("helper version mismatch: expected {expected}, got {actual}")]
    Version {
        expected: &'static str,
        actual: String,
    },
    #[error("helper protocol mismatch: expected {expected}, got {actual}")]
    Protocol { expected: u32, actual: u32 },
    #[error("helper platform mismatch: expected {expected}, got {actual}")]
    Platform { expected: String, actual: String },
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: String,
    protocol: u32,
    os: String,
    arch: String,
}

/// Shared policy plus a frozen command/cache selection. Explicit JSON retains legacy arguments.
#[derive(Debug, Clone)]
pub struct LocalHelper {
    command: HelperCommand,
    policy: LocalPolicy,
    compatibility: bool,
}

impl LocalHelper {
    pub fn from_env() -> Result<Self, DiscoveryError> {
        let (command, policy, compatibility) = match std::env::var("SPUR_EMBEDDING_HELPER_CONFIG") {
            Ok(raw) => {
                let config = HelperConfig::from_json(&raw)?;
                let policy = config.policy();
                (
                    HelperCommand::new(
                        config.executable,
                        config.args.into_iter().map(Into::into).collect(),
                    ),
                    policy,
                    false,
                )
            }
            Err(std::env::VarError::NotPresent) => {
                let path = match std::env::var_os("SPUR_EMBEDDING_HELPER") {
                    Some(value) => PathBuf::from(value),
                    None => {
                        let host = std::env::current_exe().map_err(|e| DiscoveryError::Path {
                            path: PathBuf::new(),
                            reason: e.to_string(),
                        })?;
                        host.with_file_name(if cfg!(windows) {
                            "spur-embedding-worker.exe"
                        } else {
                            "spur-embedding-worker"
                        })
                    }
                };
                if !path.is_absolute() || path.as_os_str().is_empty() {
                    return Err(DiscoveryError::Path {
                        path,
                        reason: "expected a nonempty absolute executable".into(),
                    });
                }
                let metadata = std::fs::metadata(&path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        DiscoveryError::Missing(path.clone())
                    } else {
                        DiscoveryError::Path {
                            path: path.clone(),
                            reason: e.to_string(),
                        }
                    }
                })?;
                if !metadata.is_file() {
                    return Err(DiscoveryError::Path {
                        path,
                        reason: "expected a file".into(),
                    });
                }
                let absolute = path.canonicalize().map_err(|e| DiscoveryError::Path {
                    path,
                    reason: e.to_string(),
                })?;
                (
                    HelperCommand::new(absolute, Vec::new()),
                    LocalPolicy::default(),
                    true,
                )
            }
            Err(error) => return Err(PolicyError::Environment(error).into()),
        };
        policy.validate()?;
        let mut command = command;
        command
            .environment
            .push(("SPUR_ORT_CACHE".into(), cache_from_env()?.into_os_string()));
        Ok(Self {
            command,
            policy,
            compatibility,
        })
    }
    pub fn command(&self) -> &HelperCommand {
        &self.command
    }
    pub fn policy(&self) -> &LocalPolicy {
        &self.policy
    }

    /// Caller supplies the one effective startup deadline used by probe and descriptor.
    pub fn check_compatibility(
        &self,
        deadline: Instant,
        cancel: Cancellation,
        cleanup: Arc<CleanupOwner>,
    ) -> Result<(), DiscoveryError> {
        if !self.compatibility {
            return Ok(());
        }
        let bytes = EmbeddingClient::compatibility_output(
            &self.command,
            &self.policy,
            deadline,
            cancel,
            cleanup,
        )?;
        let actual: Metadata =
            serde_json::from_slice(&bytes).map_err(|e| DiscoveryError::Metadata(e.to_string()))?;
        if actual.version != env!("CARGO_PKG_VERSION") {
            return Err(DiscoveryError::Version {
                expected: env!("CARGO_PKG_VERSION"),
                actual: actual.version,
            });
        }
        if actual.protocol != PROTOCOL_VERSION {
            return Err(DiscoveryError::Protocol {
                expected: PROTOCOL_VERSION,
                actual: actual.protocol,
            });
        }
        if actual.os != std::env::consts::OS || actual.arch != std::env::consts::ARCH {
            return Err(DiscoveryError::Platform {
                expected: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
                actual: format!("{}/{}", actual.os, actual.arch),
            });
        }
        Ok(())
    }
}

fn valid_path(path: &Path) -> bool {
    !path.as_os_str().is_empty() && !path.as_os_str().as_encoded_bytes().contains(&0)
}

fn cache_from_env() -> Result<PathBuf, DiscoveryError> {
    if let Some(value) = std::env::var_os("SPUR_ORT_CACHE") {
        let path = PathBuf::from(value);
        if !valid_path(&path) {
            return Err(DiscoveryError::Cache("empty or invalid SPUR_ORT_CACHE"));
        }
        return if path.is_absolute() {
            Ok(path)
        } else {
            std::env::current_dir()
                .map(|base| base.join(path))
                .map_err(|_| DiscoveryError::Cache("cannot resolve relative SPUR_ORT_CACHE"))
        };
    }
    let (base, suffix) = if let Some(home) = std::env::var_os("HOME") {
        (PathBuf::from(home), ".spur/cache")
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        (PathBuf::from(xdg), "spur")
    } else {
        return Err(DiscoveryError::Cache("HOME and XDG_CACHE_HOME are absent"));
    };
    if !valid_path(&base) || !base.is_absolute() {
        return Err(DiscoveryError::Cache(
            "cache base must be nonempty and absolute",
        ));
    }
    Ok(base.join(suffix))
}
