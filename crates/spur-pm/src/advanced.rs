//! Beads-only extension surface.
//!
//! These methods expose `br` CLI primitives that have no GitHub-backend
//! analog (ready, comment CRUD, dep cycles). Only `BeadsAdapter`
//! implements this trait. Callers obtain a `&dyn BeadsAdvanced` from
//! `PmService::advanced()`, which returns `None` for non-beads backends.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::sync::DepHint;
use crate::types::IssueSummary;

// ─── Filter & input types ─────────────────────────────────────────────

/// Filter passed to `BeadsAdvanced::list_ready`. Mirrors the actual flag
/// surface of `br ready` as of br 0.1.14 rather than inventing a
/// caller-convenient shape that lies about the backend's semantics.
///
/// `priorities` is a **set-membership** filter matching br's empirically
/// verified `-p, --priority <PRIORITY>  (can be repeated, 0-4 or P0-P4)`
/// model: `br ready -p 0 -p 2` returns P0 ∪ P2. Empty vec = no priority
/// filter. To express a contiguous range, enumerate:
/// `priorities: vec![2, 3, 4]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadyFilter {
    pub assignee: Option<String>,
    pub labels_all: Vec<String>,
    pub labels_any: Vec<String>,
    pub issue_type: Option<String>,
    /// Set of priorities to include (repeated `-p <n>` flags). Empty = no filter.
    pub priorities: Vec<i32>,
    pub limit: Option<usize>,
}

// ─── Output types ─────────────────────────────────────────────────────

pub type CommentId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub body: String,
    pub actor: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCycle {
    /// Issue IDs forming the cycle, in dependency order.
    pub issues: Vec<String>,
}

/// A dep hint parsed from a `spur-dep-hint v1` sentinel, with a
/// live-resolved local `beads_id` if the referenced remote node has
/// already been ingested. Read-only; the brain consumes these and
/// decides whether to call `IssueTracker::add_dependency`.
#[derive(Debug, Clone)]
pub struct ResolvedDepHint {
    pub hint: DepHint,
    pub resolved_beads_id: Option<String>,
}

/// Backend-owned change token. Consumers must only clone and return it to the
/// same backend; its encoding is not a public revision or summary-cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HygieneCursor(pub String);

/// An eligible open issue and its structural parents from the feed snapshot.
/// Parent IDs include closed parents and exclude ordinary blocking edges.
#[derive(Debug, Clone)]
pub struct HygieneCandidate {
    pub issue: IssueSummary,
    pub parent_ids: Vec<String>,
}

/// One coherent hygiene snapshot. A reseed supplies every eligible open issue;
/// otherwise candidates contain only affected eligible issues. `affected_ids`
/// also contains removed/ineligible IDs so consumers can evict pending timers.
/// Parent changes expand to children before eligibility filtering. Empty delta
/// candidates are distinct from unsupported tracking and from read errors.
#[derive(Debug, Clone)]
pub struct HygieneBatch {
    pub cursor: HygieneCursor,
    pub reseed: bool,
    pub affected_ids: Vec<String>,
    pub candidates: Vec<HygieneCandidate>,
}

// ─── Trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait BeadsAdvanced: Send + Sync {
    /// Optional complete issue/comment/structural change feed for index hygiene.
    /// None means unsupported: perform the full sweep. Errors must be retried.
    /// Publish the returned cursor only after ALL work succeeds; never replace
    /// it with a newer cursor obtained after hygiene's own writes. Cancellation
    /// or partial failure must retain the prior cursor. Timers remain independent.
    async fn hygiene_changes(
        &self,
        _since: Option<&HygieneCursor>,
    ) -> anyhow::Result<Option<HygieneBatch>> {
        Ok(None)
    }

    async fn list_ready(&self, filter: ReadyFilter) -> anyhow::Result<Vec<IssueSummary>>;

    async fn list_comments(&self, issue_id: &str) -> anyhow::Result<Vec<Comment>>;

    async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<CommentId>;

    async fn remove_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()>;

    async fn dep_cycles(&self) -> anyhow::Result<Vec<DependencyCycle>>;

    /// List dep-hint sentinels on an issue, with live resolution
    /// against the current `external_ref` index (§5.6). Read-only;
    /// never mutates. The default impl returns an empty vec so
    /// non-Beads backends compile; Beads-backed implementations
    /// override with the real read.
    async fn list_dep_hints(&self, _issue_id: &str) -> anyhow::Result<Vec<ResolvedDepHint>> {
        Ok(Vec::new())
    }
}

// ─── Unit tests for type serialization ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_filter_default_is_empty() {
        let f = ReadyFilter::default();
        assert!(f.assignee.is_none());
        assert!(f.labels_all.is_empty());
        assert!(f.labels_any.is_empty());
        assert!(f.priorities.is_empty());
        assert!(f.limit.is_none());
    }
}
