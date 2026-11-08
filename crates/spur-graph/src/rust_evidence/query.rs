//! Whole-file projections of recorded Rust evidence.

use std::{
    collections::{BTreeSet, HashSet},
    path::{Component, Path},
};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};

use super::{RustEvidenceFile, RustEvidenceOrigin};
use crate::{GraphIndexArtifact, RustCallEvidence, RustDeclarationAvailability};

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

impl RustEvidenceQueryResult {
    /// Replace whole query groups without constructing a synthetic stored set.
    /// Each input has already undergone its backend's stored validation.
    pub(crate) fn overlay(
        mut self,
        delta: Self,
        shadowed: &HashSet<String>,
    ) -> anyhow::Result<Self> {
        self.files.retain(|f| !shadowed.contains(&f.file_path));
        self.files.extend(
            delta
                .files
                .into_iter()
                .filter(|f| shadowed.contains(&f.file_path)),
        );
        self.files.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        self.origins =
            super::retained_origins(&self.files, self.origins.iter().chain(&delta.origins))?;
        self.source_domain = match (self.source_domain, delta.source_domain) {
            (Some(mut base), Some(delta)) => {
                base.retain(|s| !shadowed.contains(&s.file_path));
                base.extend(
                    delta
                        .into_iter()
                        .filter(|s| shadowed.contains(&s.file_path)),
                );
                base.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                Some(base)
            }
            // Delta bindings cannot establish the extent of an unknown base.
            _ => None,
        };
        self.availability = match &self.source_domain {
            // An empty domain does not turn absent capability into a capture.
            Some(domain)
                if domain.is_empty()
                    && self.availability == RustEvidenceAvailability::Unavailable
                    && delta.availability == RustEvidenceAvailability::Unavailable =>
            {
                RustEvidenceAvailability::Unavailable
            }
            Some(domain)
                if domain
                    .iter()
                    .all(|s| s.availability == RustEvidenceAvailability::Complete) =>
            {
                RustEvidenceAvailability::Complete
            }
            Some(domain)
                if domain
                    .iter()
                    .all(|s| s.availability == RustEvidenceAvailability::Unavailable) =>
            {
                RustEvidenceAvailability::Unavailable
            }
            Some(_) => RustEvidenceAvailability::Partial,
            None if !self.files.is_empty()
                || matches!(
                    self.availability,
                    RustEvidenceAvailability::Complete | RustEvidenceAvailability::Partial
                ) =>
            {
                RustEvidenceAvailability::Partial
            }
            None => RustEvidenceAvailability::Unavailable,
        };
        Ok(self)
    }
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

/// Validated recorded coverage, independent of decoded query rows and mutable
/// source bytes. It is also retained for legacy artifacts without a carrier.
pub(crate) struct RustEvidenceQueryContext {
    domain: Vec<RustEvidenceQuerySource>,
    captured: bool,
}

fn physical_rust(path: &str) -> bool {
    !path.starts_with("external://")
        && Path::new(path).extension().is_some_and(|e| e == "rs")
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn availability(file: &RustEvidenceFile) -> RustEvidenceAvailability {
    if matches!(&file.calls, RustCallEvidence::Unknown { .. })
        || file
            .declarations
            .sources
            .iter()
            .any(|s| matches!(&s.availability, RustDeclarationAvailability::Unknown { .. }))
    {
        RustEvidenceAvailability::Partial
    } else {
        RustEvidenceAvailability::Complete
    }
}

impl RustEvidenceQueryContext {
    pub(crate) fn new(graph: &GraphIndexArtifact) -> anyhow::Result<Self> {
        let rust_set = graph.rust_evidence_set()?;
        if let Some(set) = rust_set.as_ref() {
            // Validate every source, origin and actual owner before selection.
            set.validate_stored(graph)?;
        }
        ensure!(
            graph.graph_content_hash
                == crate::compute_graph_content_hash(
                    graph
                        .file_manifests
                        .iter()
                        .map(|m| (&m.path, &m.content_oid))
                ),
            "stale graph content hash"
        );
        let mut paths = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut domain = Vec::new();
        for manifest in &graph.file_manifests {
            ensure!(
                !manifest.path.is_empty()
                    && !manifest.stable_file_id.is_empty()
                    && !manifest.content_oid.is_empty()
                    && paths.insert(&manifest.path)
                    && ids.insert(&manifest.stable_file_id),
                "duplicate or invalid graph manifest identity"
            );
            if !physical_rust(&manifest.path) || manifest.content_oid.starts_with("gitlink:") {
                continue;
            }
            let matching = graph
                .files
                .iter()
                .filter(|f| f.file_path == manifest.path)
                .collect::<Vec<_>>();
            ensure!(
                matching.len() == 1
                    && matching[0].stable_file_id == manifest.stable_file_id
                    && graph
                        .files
                        .iter()
                        .filter(|f| f.stable_file_id == manifest.stable_file_id)
                        .count()
                        == 1,
                "source is not an authoritative physical graph file"
            );
            let file = rust_set
                .as_ref()
                .and_then(|s| s.files.iter().find(|f| f.file_path == manifest.path));
            domain.push(RustEvidenceQuerySource {
                file_path: manifest.path.clone(),
                stable_file_id: manifest.stable_file_id.clone(),
                source_oid: manifest.content_oid.clone(),
                availability: file.map_or(RustEvidenceAvailability::Unavailable, availability),
            });
        }
        for file in &graph.files {
            ensure!(
                !physical_rust(&file.file_path)
                    || file.file_path.contains("#cell:")
                    || graph
                        .file_manifests
                        .iter()
                        .any(|m| m.path == file.file_path),
                "physical Rust graph file has no manifest"
            );
        }
        domain.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        Ok(Self {
            domain,
            captured: rust_set.is_some(),
        })
    }

    pub(crate) fn result(
        &self,
        filter: &RustEvidenceFilter,
    ) -> anyhow::Result<RustEvidenceQueryResult> {
        filter.validate_shape()?;
        if let Some(oid) = &filter.source_oid {
            let path = filter
                .file_path
                .as_deref()
                .context("source_oid requires file_path")?;
            let source = self
                .domain
                .iter()
                .find(|s| s.file_path == path)
                .context("source_oid has no recorded physical Rust source")?;
            ensure!(
                source.source_oid == *oid,
                "stale source_oid for {path}: expected {}, actual {oid}",
                source.source_oid
            );
        }
        let domain = self
            .domain
            .iter()
            .filter(|s| filter.file_path.as_ref().is_none_or(|p| *p == s.file_path))
            .cloned()
            .collect::<Vec<_>>();
        // An authoritative physical binding wins over notebook-like spelling.
        let unsupported = filter
            .file_path
            .as_deref()
            .is_some_and(|p| domain.is_empty() && (!physical_rust(p) || p.contains("#cell:")));
        let availability = if unsupported {
            RustEvidenceAvailability::UnsupportedSource
        } else if !self.captured
            || (filter.file_path.is_some() && domain.is_empty())
            || (!domain.is_empty()
                && domain
                    .iter()
                    .all(|s| s.availability == RustEvidenceAvailability::Unavailable))
        {
            RustEvidenceAvailability::Unavailable
        } else if domain
            .iter()
            .any(|s| s.availability != RustEvidenceAvailability::Complete)
        {
            RustEvidenceAvailability::Partial
        } else {
            RustEvidenceAvailability::Complete
        };
        Ok(RustEvidenceQueryResult {
            availability,
            source_domain: Some(domain),
            files: Vec::new(),
            origins: Vec::new(),
        })
    }

    pub(crate) fn project(
        &self,
        graph: &GraphIndexArtifact,
        filter: &RustEvidenceFilter,
    ) -> anyhow::Result<RustEvidenceQueryResult> {
        let mut result = self.result(filter)?;
        if let Some(set) = graph.rust_evidence_set()? {
            result.files = set
                .files
                .iter()
                .filter(|f| filter.file_path.as_ref().is_none_or(|p| *p == f.file_path))
                .cloned()
                .collect();
            result.origins = referenced_origins(&result.files, &set.origins);
        }
        Ok(result)
    }
}

pub(crate) fn referenced_origins(
    files: &[RustEvidenceFile],
    origins: &[RustEvidenceOrigin],
) -> Vec<RustEvidenceOrigin> {
    let used = files
        .iter()
        .map(|f| f.origin_graph_content_hash.as_str())
        .collect::<BTreeSet<_>>();
    origins
        .iter()
        .filter(|o| used.contains(o.graph_content_hash.as_str()))
        .cloned()
        .collect()
}
