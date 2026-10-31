//! Identity for a closed, explicitly inventoried fixture capture. The producer
//! must clear ambient Cargo configuration/environment, inventory all source,
//! manifest, lock and config files (including local dependencies), and check
//! bytes before/after capture. This metadata API cannot prove that inventory or
//! that a declared binary actually ran; an independently retained expected ID
//! and the P3 capture procedure are required. Arbitrary ambient builds are out
//! of scope. Paths are physical, root-relative Unix paths, with no remapping.
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CAPTURE_BUILD_SCHEMA_VERSION: &str = "panic-cga-capture-v1";
pub const CGA_REVISION: &str = "2fc63108f0b674fe7a178191ffa15647a18879f1";
pub const CGA_TOOLCHAIN: &str = "nightly-2026-10-18";
/// Every key must be present, including an explicit None for an unset value.
/// All other semantic environment variables must be cleared by the producer;
/// required process plumbing (PATH, isolated Cargo/Rustup homes, wrapper and
/// exporter paths) belongs in the recorded capture command/input inventory.
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
    /// Exact rustc -vV output, not a parsed release number alone.
    pub rustc_verbose: String,
    pub toolchain: String,
    pub host: String,
    pub target: String,
    pub exporter_revision: String,
    pub exporter_binary_sha256: String,
    /// Sets; canonicalized for hashing, with duplicate members rejected.
    pub packages: Vec<String>,
    pub targets: Vec<String>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub all_features: bool,
    pub profile: String,
    /// Ordered argv entries; never sorted, split or shell-interpreted.
    pub cfg_flags: Vec<String>,
    pub compiler_flags: Vec<String>,
    pub cargo_args: Vec<String>,
    pub environment: BTreeMap<String, Option<String>>,
    pub capture_command: Vec<String>,
    pub raw_export_sha256: String,
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl CaptureBuildIdentity {
    /// Includes export bytes through their checksum. Even a semantically equal
    /// reserialization of an export requires a new capture identity.
    pub fn build_id(&self) -> anyhow::Result<String> {
        let canonical = self.canonical()?;
        let mut hash = Sha256::new();
        hash.update(b"spur-panic-cga-capture-v1\0");
        hash.update(serde_json::to_vec(&canonical)?);
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn validate_inputs(&self, inputs: &[CaptureInputBytes<'_>]) -> anyhow::Result<()> {
        self.canonical()?;
        let mut supplied = BTreeMap::new();
        for input in inputs {
            ensure!(
                supplied.insert(input.path, input.bytes).is_none(),
                "duplicate capture input bytes"
            );
        }
        ensure!(
            supplied.len() == self.inputs.len(),
            "capture input inventory cardinality mismatch"
        );
        for input in &self.inputs {
            let bytes = supplied
                .get(input.path.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing capture input: {}", input.path))?;
            ensure!(
                sha256_bytes(bytes) == input.sha256,
                "capture input checksum mismatch: {}",
                input.path
            );
        }
        Ok(())
    }

    pub(super) fn canonical(&self) -> anyhow::Result<Self> {
        ensure!(
            self.schema_version == CAPTURE_BUILD_SCHEMA_VERSION,
            "unsupported capture schema"
        );
        ensure!(
            self.exporter_revision == CGA_REVISION && self.toolchain == CGA_TOOLCHAIN,
            "unsupported exporter revision/toolchain"
        );
        ensure!(
            valid_sha256(&self.graph_content_hash)
                && valid_sha256(&self.exporter_binary_sha256)
                && valid_sha256(&self.raw_export_sha256),
            "missing or invalid capture digest"
        );
        ensure!(
            self.capture_root.starts_with('/')
                && (self.capture_root == "/" || normal_relative_path(&self.capture_root[1..])),
            "capture root must be a canonical absolute Unix path"
        );
        for value in [&self.host, &self.target, &self.profile, &self.rustc_verbose] {
            ensure!(
                !value.trim().is_empty() && !value.contains('\0'),
                "missing capture identity field"
            );
        }
        // Keep the complete output; check the required verbose identity fields
        // without inventing a rustc release or substituting the toolchain label.
        ensure!(
            self.rustc_verbose
                .lines()
                .next()
                .is_some_and(|l| l.starts_with("rustc ")),
            "missing verbose rustc identity"
        );
        for key in [
            "binary",
            "commit-hash",
            "commit-date",
            "host",
            "release",
            "LLVM version",
        ] {
            let prefix = format!("{key}: ");
            let values: Vec<_> = self
                .rustc_verbose
                .lines()
                .filter_map(|line| line.strip_prefix(&prefix))
                .collect();
            ensure!(
                values.len() == 1 && !values[0].trim().is_empty(),
                "missing/duplicate rustc identity component: {key}"
            );
            if key == "host" {
                ensure!(values[0] == self.host, "rustc host identity mismatch");
            }
            if key == "release" {
                ensure!(
                    values[0].contains("nightly"),
                    "pinned exporter requires nightly rustc"
                );
            }
        }
        ensure!(
            !self.packages.is_empty()
                && !self.targets.is_empty()
                && !self.capture_command.is_empty()
                && !self.cargo_args.is_empty(),
            "missing bounded package/target/command selection"
        );
        for arguments in [
            &self.cfg_flags,
            &self.compiler_flags,
            &self.cargo_args,
            &self.capture_command,
        ] {
            ensure!(
                arguments.iter().all(|a| !a.contains('\0')),
                "NUL in recorded argv"
            );
        }
        ensure!(
            !self.capture_command[0].is_empty() && !self.cargo_args[0].is_empty(),
            "missing capture executable/subcommand"
        );
        let allowed: BTreeSet<_> = CAPTURE_ENV_ALLOWLIST.iter().copied().collect();
        ensure!(
            self.environment
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == allowed,
            "capture environment must exactly record the allowlist, including unset keys"
        );
        ensure!(
            self.environment
                .values()
                .flatten()
                .all(|v| !v.contains('\0')),
            "NUL in captured environment"
        );
        let mut canonical = self.clone();
        for set in [
            &mut canonical.packages,
            &mut canonical.targets,
            &mut canonical.features,
        ] {
            ensure!(
                set.iter()
                    .all(|v| !v.trim().is_empty() && !v.contains('\0')),
                "empty capture set member"
            );
            set.sort();
            ensure!(
                set.windows(2).all(|w| w[0] != w[1]),
                "duplicate capture set member"
            );
        }
        let mut paths = BTreeSet::new();
        for input in &self.inputs {
            ensure!(
                normal_relative_path(&input.path) && valid_sha256(&input.sha256),
                "invalid capture input path/checksum"
            );
            ensure!(paths.insert(&input.path), "duplicate capture input path");
        }
        for kind in [
            CaptureInputKind::Source,
            CaptureInputKind::Manifest,
            CaptureInputKind::Lockfile,
        ] {
            ensure!(
                self.inputs.iter().any(|i| i.kind == kind),
                "missing required capture input kind: {kind:?}"
            );
        }
        canonical
            .inputs
            .sort_by(|a, b| (&a.path, a.kind).cmp(&(&b.path, b.kind)));
        Ok(canonical)
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn normal_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\0', '\\'])
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}
