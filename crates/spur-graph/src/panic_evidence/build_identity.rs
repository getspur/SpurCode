use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CAPTURE_BUILD_SCHEMA_VERSION: &str = "panic-cga-capture-v1";
pub const CGA_REVISION: &str = "2fc63108f0b674fe7a178191ffa15647a18879f1";
pub const CGA_TOOLCHAIN: &str = "nightly-2026-10-18";
pub const CAPTURE_ENV_ALLOWLIST: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_INCREMENTAL",
    "CARGO_PROFILE_DEV_OPT_LEVEL",
    "CARGO_PROFILE_DEV_DEBUG_ASSERTIONS",
    "CARGO_PROFILE_DEV_OVERFLOW_CHECKS",
    "CARGO_PROFILE_DEV_PANIC",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScope {
    BoundedFixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportProvenance {
    SyntheticFixture,
    CompilerCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureInputKind {
    Source,
    Manifest,
    Lockfile,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureInput {
    pub path: String,
    pub kind: CaptureInputKind,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureInputBytes<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBuildIdentity {
    pub schema_version: String,
    pub scope: CaptureScope,
    pub provenance: ExportProvenance,
    pub graph_content_hash: String,
    pub capture_root: String,
    pub inputs: Vec<CaptureInput>,
    pub rustc_verbose: String,
    pub toolchain: String,
    pub host: String,
    pub target: String,
    pub exporter_revision: String,
    pub exporter_binary_sha256: String,
    pub packages: Vec<String>,
    pub targets: Vec<String>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub all_features: bool,
    pub profile: String,
    pub cfg_flags: Vec<String>,
    pub compiler_flags: Vec<String>,
    pub cargo_args: Vec<String>,
    pub environment: BTreeMap<String, Option<String>>,
    pub capture_command: Vec<String>,
    pub raw_export_sha256: String,
}

pub fn sha256_bytes(_bytes: &[u8]) -> String {
    String::new()
}

impl CaptureBuildIdentity {
    pub fn build_id(&self) -> anyhow::Result<String> {
        Ok(String::new())
    }
    pub fn validate_inputs(&self, _inputs: &[CaptureInputBytes<'_>]) -> anyhow::Result<()> {
        Ok(())
    }
}
