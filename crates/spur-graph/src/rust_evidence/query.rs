//! Whole-file projections of recorded Rust evidence.

use anyhow::ensure;
use serde::{Deserialize, Serialize};

use super::{RustEvidenceFile, RustEvidenceOrigin};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceFilter {
    pub file_path: Option<String>,
    pub source_oid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RustEvidenceAvailability {
    Complete,
    Partial,
    Unavailable,
    UnsupportedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceQuerySource {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub availability: RustEvidenceAvailability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceQueryResult {
    pub availability: RustEvidenceAvailability,
    pub source_domain: Option<Vec<RustEvidenceQuerySource>>,
    pub files: Vec<RustEvidenceFile>,
    pub origins: Vec<RustEvidenceOrigin>,
}

impl RustEvidenceFilter {
    pub fn validate_shape(&self) -> anyhow::Result<()> {
        ensure!(
            self.source_oid.is_none() || self.file_path.is_some(),
            "source_oid requires file_path"
        );
        Ok(())
    }
}
