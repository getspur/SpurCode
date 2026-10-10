//! Selective reuse for recurring, complete issue-summary memberships.
//!
//! Admission: unpaginated, since=None, text_search=None queries with ordinary
//! INTEGER priority/TEXT creation keys. Pages (including limit=0), timestamp
//! queries, ad hoc text searches and unusual ordering storage classes use S3
//! SQL. In particular, SQL LIMIT runs before decoded-tombstone removal in S3:
//! caching an unlimited projection for those pages would broaden error scope.
//!
//! Retention uses the connection's configured SQLite cache_size/page_size byte
//! budget and at most DEFAULT_READER_THREADS subscriptions. Keys and snapshots
//! are boxed serialized bytes, making retained dynamic bytes exact. The fixed
//! slot array is O(reader count). Request/response and serialization scratch
//! remain proportional to the requested result, as on the direct path. No TTL.
//!
//! A single adapter-owned mutex, acquired on the existing blocking reader pool,
//! coalesces refreshes. Each retained snapshot includes database identity/schema
//! generation in its IssueCursor and the full normalized filter/order/projection
//! in its key. The feed reads rows and cursor in one transaction; validation
//! after loading records an ordinary revision race without advancing that
//! snapshot's cursor. A known identity/schema/generation reset or coverage gap
//! instead discards the candidate and returns an error; validation errors also
//! propagate. The next request seeds the discarded subscription anew. A raced
//! response is a coherent earlier snapshot, explicitly dirty, and the next
//! reader consumes the intervening delta. No claim of unbounded freshness
//! or of preventing commits after the validation linearization point is made.

use std::path::Path;
use std::str::FromStr;

use beads_rust::model::{IssueType, Status};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::beads_db::DEFAULT_READER_THREADS;
use super::issue_changes::{IssueChangeFeed, IssueCursor};
use super::summary_query::{query_summaries, QueryWork, SummaryRow};
use crate::types::{IssueFilter, IssueSummary};

/// Cumulative database work, plus current retained dynamic storage. Row loads
/// count actual summary/label decoding, not invocations or rows returned from
/// memory. Candidate probes inspect indexed membership predicates only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IssueReadWork {
    pub full_builds: u64,
    pub delta_refreshes: u64,
    pub reuses: u64,
    pub direct_reads: u64,
    /// Loaded results returned without retention because their serialized key
    /// and snapshot exceed the SQLite-derived budget. Counts once per request.
    pub over_budget_bypasses: u64,
    /// Loaded results returned without retention because SQL ordering cannot
    /// be reproduced by OrderKey. Counts once per request, including delta SQL
    /// reloads already recorded in direct_reads. Checked before the byte budget.
    pub unsupported_order_bypasses: u64,
    pub candidate_ids: u64,
    pub issue_rows: u64,
    pub label_rows: u64,
    pub races: u64,
    pub connection_opens: u64,
    pub retained_bytes: usize,
    pub budget_bytes: usize,
    pub subscriptions: usize,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    cursor: IssueCursor,
    rows: Vec<SummaryRow>,
    clean: bool,
}
struct Subscription {
    key: Box<str>,
    snapshot: Box<[u8]>,
}
impl Subscription {
    fn bytes(&self) -> usize {
        self.key.len().saturating_add(self.snapshot.len())
    }
}

pub(crate) struct IssueReads {
    feed: IssueChangeFeed,
    // Oldest first, fixed maximum capacity derived from the existing pool.
    subscriptions: Vec<Subscription>,
    pub(crate) work: IssueReadWork,
}
impl IssueReads {
    pub(crate) fn new(path: &Path, timeout_ms: u64) -> Self {
        Self {
            feed: IssueChangeFeed::new(path, timeout_ms),
            subscriptions: Vec::with_capacity(DEFAULT_READER_THREADS),
            work: IssueReadWork::default(),
        }
    }

    pub(crate) fn hygiene(
        &mut self,
        since: Option<&crate::advanced::HygieneCursor>,
    ) -> anyhow::Result<crate::advanced::HygieneBatch> {
        let result = super::hygiene::read(&mut self.feed, since);
        self.work.connection_opens = self.feed.connection_opens;
        result
    }

    pub(crate) fn list(&mut self, filter: &IssueFilter) -> anyhow::Result<Vec<IssueSummary>> {
        let mut work = IssueReadWork::default();
        let mut rows = QueryWork::default();
        let result = self.list_inner(filter, &mut work, &mut rows);
        work.issue_rows = rows.issues;
        work.label_rows = rows.labels;
        self.work.full_builds += work.full_builds;
        self.work.delta_refreshes += work.delta_refreshes;
        self.work.reuses += work.reuses;
        self.work.direct_reads += work.direct_reads;
        self.work.over_budget_bypasses += work.over_budget_bypasses;
        self.work.unsupported_order_bypasses += work.unsupported_order_bypasses;
        self.work.candidate_ids += work.candidate_ids;
        self.work.issue_rows += work.issue_rows;
        self.work.label_rows += work.label_rows;
        self.work.races += work.races;
        self.work.connection_opens = self.feed.connection_opens;
        self.work.subscriptions = self.subscriptions.len();
        self.work.retained_bytes = self.subscriptions.iter().map(Subscription::bytes).sum();
        tracing::info!(
            target: "issue_probe", site = "beads_issue_reads",
            full_builds = work.full_builds, delta_refreshes = work.delta_refreshes,
            reuses = work.reuses, direct_reads = work.direct_reads,
            over_budget_bypasses = work.over_budget_bypasses,
            unsupported_order_bypasses = work.unsupported_order_bypasses,
            candidate_ids = work.candidate_ids, issue_rows = work.issue_rows,
            label_rows = work.label_rows, races = work.races,
            subscriptions = self.work.subscriptions,
            retained_bytes = self.work.retained_bytes, budget_bytes = self.work.budget_bytes,
            succeeded = result.is_ok(), "issue subscription work",
        );
        result
    }

    fn list_inner(
        &mut self,
        filter: &IssueFilter,
        work: &mut IssueReadWork,
        rows: &mut QueryWork,
    ) -> anyhow::Result<Vec<IssueSummary>> {
        let Some(normalized) = normalized(filter) else {
            work.direct_reads += 1;
            return self.direct(filter, rows);
        };
        // A versioned projection/order identity accompanies every serialized
        // field of IssueFilter; no free-text values are emitted in tracing.
        let key = serde_json::to_string(&("summary_v1:priority_asc_created_desc", &normalized))?;
        let previous = self
            .subscriptions
            .iter()
            .position(|s| s.key.as_ref() == key)
            .map(|i| self.subscriptions.remove(i));
        let mut snapshot = previous
            .as_ref()
            .map(|s| serde_json::from_slice::<Snapshot>(&s.snapshot))
            .transpose()?;
        let previous_clean = snapshot.as_ref().map(|s| s.clean);
        let since = snapshot.as_ref().map(|s| s.cursor.clone());
        let loaded = self.feed.read(since.as_ref(), |tx, changes| {
            let budget = retention_budget(tx)?;
            let mut candidate = if changes.reseed {
                work.full_builds += 1;
                Snapshot {
                    cursor: changes.cursor.clone(),
                    rows: query_summaries(tx, &normalized, None, true, rows)?,
                    clean: false,
                }
            } else {
                let mut previous = snapshot.take().expect("non-reseed requires a cursor");
                let mut relevant = false;
                for change in changes.changes.iter().filter(|c| c.summary) {
                    work.candidate_ids += 1;
                    let old_len = previous.rows.len();
                    previous
                        .rows
                        .retain(|row| row.summary.id != change.issue_id);
                    let new_rows = if change.deleted {
                        Vec::new()
                    } else {
                        query_summaries(tx, &normalized, Some(&change.issue_id), true, rows)?
                    };
                    // Both old and new membership matter: deletion/exit removes
                    // old members; a newly matching ID was never in the cache.
                    relevant |= old_len != previous.rows.len() || !new_rows.is_empty();
                    previous.rows.extend(new_rows);
                }
                if relevant {
                    work.delta_refreshes += 1;
                    if previous.rows.iter().all(|r| r.order.is_some()) {
                        previous.rows.sort_by(|a, b| a.order.cmp(&b.order));
                    } else {
                        // Exceptional storage class: retain the exact SQL order.
                        work.direct_reads += 1;
                        previous.rows = query_summaries(tx, &normalized, None, true, rows)?;
                    }
                } else {
                    work.reuses += 1;
                }
                previous.cursor = changes.cursor.clone();
                previous
            };
            candidate.clean = false;
            Ok((candidate, budget))
        });
        let (mut candidate, budget) = match loaded {
            Ok(value) => value,
            Err(error)
                if error
                    .downcast_ref::<beads_rust::error::BeadsError>()
                    .is_some() =>
            {
                return Err(error)
            }
            Err(error) => {
                // A broken source table can prevent feed repair. Run the S3
                // query on a replacement-safe connection so its original typed
                // error survives; successful untracked reads never seed reuse.
                tracing::debug!(target: "issue_probe", %error, "issue feed unavailable; direct summary read");
                work.direct_reads += 1;
                return self.direct(filter, rows);
            }
        };
        self.work.budget_bytes = budget;
        tracing::debug!(
            target: "issue_probe", site = "beads_issue_reads_validate",
            "issue subscription snapshot loaded; validating cursor",
        );
        // Only a same-epoch revision race may return an earlier coherent
        // snapshot. A reset invalidates that justification; errors do not give
        // us any validation evidence. The old subscription was removed above,
        // so either error discards this candidate and the next request reseeds.
        // The outer list method still records all work spent on this attempt.
        candidate.clean = self.feed.read(Some(&candidate.cursor), |_, changes| {
            anyhow::ensure!(
                !changes.reseed,
                "issue subscription invalidated during validation; discard result and retry"
            );
            Ok(changes.cursor == candidate.cursor)
        })?;
        if !candidate.clean {
            // Keep the ORIGINAL cursor: validation's newer cursor belongs to
            // different rows, even when its commit was unrelated to this query.
            work.races += 1;
        }
        let cacheable = candidate.rows.iter().all(|r| r.order.is_some());
        if cacheable {
            let retained = if since.as_ref() == Some(&candidate.cursor)
                && previous_clean == Some(candidate.clean)
            {
                // The complete validated feed observed no intervening revision.
                // Reuse exact boxed bytes; output decoding still belongs to this
                // request. The pager budget and eviction policy still run below.
                previous.expect("unchanged cursor requires a previous subscription")
            } else {
                tracing::debug!(target: "issue_probe", site = "beads_issue_snapshot_serialize", "serialize issue snapshot for retention");
                Subscription {
                    key: key.into_boxed_str(),
                    snapshot: serde_json::to_vec(&candidate)?.into_boxed_slice(),
                }
            };
            if retained.bytes() <= budget {
                while self.subscriptions.len() >= DEFAULT_READER_THREADS
                    || self
                        .subscriptions
                        .iter()
                        .map(Subscription::bytes)
                        .sum::<usize>()
                        .saturating_add(retained.bytes())
                        > budget
                {
                    self.subscriptions.remove(0);
                }
                self.subscriptions.push(retained);
            } else {
                work.over_budget_bypasses += 1;
            }
        } else {
            work.unsupported_order_bypasses += 1;
        }
        // A reduced pager budget also evicts old subscriptions when this result
        // itself is oversized/non-admissible.
        while self
            .subscriptions
            .iter()
            .map(Subscription::bytes)
            .sum::<usize>()
            > budget
        {
            self.subscriptions.remove(0);
        }
        Ok(candidate.rows.into_iter().map(|r| r.summary).collect())
    }

    fn direct(
        &mut self,
        filter: &IssueFilter,
        work: &mut QueryWork,
    ) -> anyhow::Result<Vec<IssueSummary>> {
        self.feed.read_direct(|tx| {
            Ok(query_summaries(tx, filter, None, false, work)?
                .into_iter()
                .map(|r| r.summary)
                .collect())
        })
    }
}

fn normalized(filter: &IssueFilter) -> Option<IssueFilter> {
    if !cfg!(unix)
        || filter.limit.is_some()
        || filter.offset.unwrap_or(0) != 0
        || filter.since.is_some()
        || filter.text_search.is_some()
    {
        return None;
    }
    let mut f = filter.clone();
    f.offset = None;
    f.labels.sort();
    f.labels.dedup();
    f.status = f.status.as_deref().map(|s| {
        Status::from_str(s)
            .unwrap_or(Status::Open)
            .as_str()
            .to_owned()
    });
    f.issue_type = f.issue_type.as_deref().map(|s| {
        IssueType::from_str(s)
            .unwrap_or(IssueType::Task)
            .as_str()
            .to_owned()
    });
    if f.status.is_some() {
        f.include_closed = true;
    }
    match (f.priority_min, f.priority_max) {
        (None, None) => {}
        (min, max) => {
            let (min, max) = (min.unwrap_or(0), max.unwrap_or(4));
            if min > max {
                f.priority_min = None;
                f.priority_max = None;
            } else {
                f.priority_min = Some(min);
                f.priority_max = Some(max);
            }
        }
    }
    Some(f)
}

fn retention_budget(conn: &Connection) -> anyhow::Result<usize> {
    let pages: i64 = conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;
    let page_bytes: u64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let bytes = pages
        .unsigned_abs()
        .saturating_mul(if pages < 0 { 1024 } else { page_bytes });
    Ok(usize::try_from(bytes).unwrap_or(usize::MAX))
}
