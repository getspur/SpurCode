//! Hygiene cursors are independent of summary subscriptions. All candidate and
//! parent expansion SQL belongs to the feed transaction, before any repairs.
use std::collections::BTreeSet;

use super::issue_changes::{IssueChangeFeed, IssueCursor};
use super::summary_query::{query_summaries, QueryWork};
use crate::advanced::{HygieneBatch, HygieneCandidate, HygieneCursor};
use crate::IssueFilter;

const CHILDREN: &str =
    "SELECT issue_id FROM dependencies WHERE depends_on_id=?1 AND LOWER(type)='parent-child'";

// Match graph endpoint eligibility and DependencyType's case normalization.
// Closed/custom/NULL statuses remain eligible; whitespace is significant.
const PARENTS: &str = "SELECT d.depends_on_id FROM dependencies d
    JOIN issues p ON p.id=d.depends_on_id
    WHERE d.issue_id=?1 AND LOWER(d.type)='parent-child'
      AND COALESCE(LOWER(p.status),'open')<>'tombstone'
      AND COALESCE(p.is_template,0)=0
    ORDER BY d.depends_on_id";

pub(super) fn read(
    feed: &mut IssueChangeFeed,
    since: Option<&HygieneCursor>,
) -> anyhow::Result<HygieneBatch> {
    // A token from an incompatible backend/version safely starts a new seed.
    let since = since.and_then(|c| serde_json::from_str::<IssueCursor>(&c.0).ok());
    feed.read(since.as_ref(), |tx, batch| {
        let mut affected = BTreeSet::new();
        let mut children = tx.prepare(CHILDREN)?;
        for change in batch.changes.iter().filter(|c| c.issue || c.comments) {
            affected.insert(change.issue_id.clone());
            // Indexed by idx_dependencies_depends_on_type, including closed
            // parents. Expand once (no recursive ownership or broad graph scan).
            for child in children.query_map([&change.issue_id], |r| r.get::<_, String>(0))? {
                affected.insert(child?);
            }
        }
        let filter = IssueFilter {
            status: Some("open".into()),
            ..Default::default()
        };
        let mut work = QueryWork::default();
        let rows = if batch.reseed {
            query_summaries(tx, &filter, None, false, &mut work)?
        } else {
            let mut rows = Vec::new();
            for id in &affected {
                rows.extend(query_summaries(tx, &filter, Some(id), false, &mut work)?);
            }
            rows
        };
        let mut parents = tx.prepare(PARENTS)?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let parent_ids = parents
                .query_map([&row.summary.id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            candidates.push(HygieneCandidate {
                issue: row.summary,
                parent_ids,
            });
        }
        Ok(HygieneBatch {
            cursor: HygieneCursor(serde_json::to_string(&batch.cursor)?),
            reseed: batch.reseed,
            affected_ids: affected.into_iter().collect(),
            candidates,
        })
    })
}

#[cfg(test)]
#[path = "hygiene_tests.rs"]
mod tests;
