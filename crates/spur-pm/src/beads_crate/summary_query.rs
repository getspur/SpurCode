//! Narrow list projection, matched to `beads_rust` 47b3b39 and the adapter at
//! 083241051. Keep the legacy predicate/order/window sequence: SQL limits the
//! candidate rows, decoded tombstones are removed, labels are batched, then
//! offset/take is applied. Ties have no ID tiebreaker; inverted priority ranges
//! add no predicate; a zero limit still executes SQL and loads labels.
//!
//! Only id/title/description/status/priority/type/assignee are decoded. The
//! other 29 `Issue` columns are intentionally not validated as Rust values:
//! `content_hash`, design, `acceptance_criteria`, notes, owner, `estimated_minutes`,
//! `created_at`, `created_by`, `updated_at`, `closed_at`, `close_reason`,
//! `closed_by_session`, `due_at`, `defer_until`, `external_ref`, `source_system`,
//! `source_repo`, `deleted_at`, `deleted_by`, `delete_reason`, `original_type`,
//! `compaction_level`, `compacted_at`, `compacted_at_commit`, `original_size`, sender,
//! ephemeral, pinned, `is_template`. SQL still uses stored filter/order values.
//! Projected-column and query errors retain the upstream `BeadsError` wrapper
//! and original column indices. Empty strings and status/type parsing follow
//! upstream normalization exactly.
//!
//! The caller owns the connection and any transaction. Like the legacy list,
//! this helper does not start a transaction across issue and label queries;
//! callers needing one coherent snapshot must supply their own transaction.
//! No results, watermarks, or subscriptions are cached here.

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Instant;

use beads_rust::error::BeadsError;
use beads_rust::model::{IssueType, Status};
use rusqlite::{Connection, ToSql};

use crate::types::{IssueFilter, IssueSummary, PmSource};

/// Actual SQL rows decoded, independent of returned window size.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct QueryWork {
    pub(crate) issues: u64,
    pub(crate) labels: u64,
}

/// SQLite's order for the admitted ordinary INTEGER/TEXT storage classes.
/// Ties remain unspecified by the public API; rowid matches the existing
/// priority/creation indexes without adding an SQL or public ID tiebreaker.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct OrderKey {
    priority: Option<i32>,
    created: Option<String>,
    rowid: i64,
}
impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.created.cmp(&self.created))
            .then_with(|| self.rowid.cmp(&other.rowid))
    }
}
impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SummaryRow {
    pub(crate) summary: IssueSummary,
    pub(crate) order: Option<OrderKey>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "S3 connection-based compatibility API")
)]
pub(crate) fn list_summaries(
    conn: &Connection,
    filter: &IssueFilter,
) -> anyhow::Result<Vec<IssueSummary>> {
    Ok(
        query_summaries(conn, filter, None, false, &mut QueryWork::default())?
            .into_iter()
            .map(|r| r.summary)
            .collect(),
    )
}

/// Add a primary-key candidate restriction before the unchanged S3 predicates.
/// Nonmembers never reach the summary decoder or batched label loader.
pub(crate) fn query_summaries(
    conn: &Connection,
    filter: &IssueFilter,
    id: Option<&str>,
    ordered: bool,
    work: &mut QueryWork,
) -> anyhow::Result<Vec<SummaryRow>> {
    let mut sql =
        String::from("SELECT id, title, description, status, priority, issue_type, assignee");
    if ordered {
        sql.push_str(", rowid, priority, created_at");
    }
    sql.push_str(" FROM issues WHERE 1=1");
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(id) = id {
        sql.push_str(" AND id = ?");
        params.push(Box::new(id.to_owned()));
    }
    let status = filter
        .status
        .as_deref()
        .map(|s| Status::from_str(s).unwrap_or(Status::Open));
    if let Some(status) = &status {
        sql.push_str(" AND status IN (?)");
        params.push(Box::new(status.as_str().to_owned()));
    }
    if let Some(kind) = &filter.issue_type {
        sql.push_str(" AND issue_type IN (?)");
        params.push(Box::new(
            IssueType::from_str(kind)
                .unwrap_or(IssueType::Task)
                .as_str()
                .to_owned(),
        ));
    }
    let priorities = match (filter.priority_min, filter.priority_max) {
        (Some(min), max) => Some(min..=max.unwrap_or(4)),
        (None, Some(max)) => Some(0..=max),
        (None, None) => None,
    };
    if let Some(priorities) = priorities.filter(|range| !range.is_empty()) {
        let values: Vec<_> = priorities.collect();
        sql.push_str(" AND priority IN (");
        sql.push_str(&vec!["?"; values.len()].join(","));
        sql.push(')');
        params.extend(values.into_iter().map(|v| Box::new(v) as Box<dyn ToSql>));
    }
    if let Some(assignee) = &filter.assignee {
        sql.push_str(" AND assignee = ?");
        params.push(Box::new(assignee.clone()));
    }
    if !filter.include_closed && filter.status.is_none() {
        sql.push_str(" AND status NOT IN ('closed', 'tombstone', 'deferred')");
    }
    sql.push_str(" AND (is_template = 0 OR is_template IS NULL)");
    for label in &filter.labels {
        // Retain the correlated AND predicates: a blanket ID-subquery rewrite
        // regressed broad labels in the measured proposal.
        sql.push_str(" AND EXISTS (SELECT 1 FROM labels WHERE labels.issue_id = issues.id AND labels.label = ?)");
        params.push(Box::new(label.clone()));
    }
    if let Some(text) = &filter.text_search {
        sql.push_str(" AND title LIKE ? ESCAPE '\\'");
        let escaped = text
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        params.push(Box::new(format!("%{escaped}%")));
    }
    if let Some(since) = filter.since {
        sql.push_str(" AND updated_at >= ?");
        params.push(Box::new(since.to_rfc3339()));
    }
    sql.push_str(" ORDER BY priority ASC, created_at DESC");
    let offset = filter.offset.unwrap_or(0);
    if let Some(limit) = filter.limit.map(|n| n.saturating_add(offset)) {
        if limit > 0 {
            sql.push_str(" LIMIT ?");
            params.push(Box::new(limit));
        }
    }

    let mut sql_decode_us = 0;
    let mut labels_us = 0;
    let mut summary_us = 0;
    let mut sql_rows = 0;
    let mut label_rows = 0;
    let mut returned_rows = 0;
    let mut stage = "sql";
    let result = (|| -> Result<Vec<SummaryRow>, BeadsError> {
        let started = Instant::now();
        let result = (|| -> rusqlite::Result<Vec<SummaryRow>> {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(&params), |row| {
                sql_rows += 1;
                work.issues += 1;
                let summary = summary_from_row(row).map_err(legacy_column_error)?;
                let order = if ordered { order_from_row(row)? } else { None };
                Ok(SummaryRow { summary, order })
            })?;
            rows.collect()
        })();
        sql_decode_us = started.elapsed().as_micros() as u64;
        let mut summaries = result?;

        let started = Instant::now();
        if status != Some(Status::Tombstone) {
            summaries.retain(|issue| issue.summary.status != "tombstone");
        }
        let ids: Vec<_> = summaries
            .iter()
            .map(|issue| issue.summary.id.clone())
            .collect();
        summary_us = started.elapsed().as_micros() as u64;

        stage = "labels";
        let started = Instant::now();
        let labels = labels_for_issues(conn, &ids, work);
        labels_us = started.elapsed().as_micros() as u64;
        let mut labels = labels?;
        label_rows = labels.values().map(Vec::len).sum::<usize>();

        let started = Instant::now();
        for summary in &mut summaries {
            summary.summary.labels = labels.remove(&summary.summary.id).unwrap_or_default();
        }
        let rows = summaries.into_iter().skip(offset);
        let summaries: Vec<_> = match filter.limit {
            Some(limit) => rows.take(limit).collect(),
            None => rows.collect(),
        };
        summary_us += started.elapsed().as_micros() as u64;
        returned_rows = summaries.len();
        stage = "complete";
        Ok(summaries)
    })();
    tracing::info!(
        target: "issue_probe",
        site = "beads_list_issues",
        sql_decode_us,
        labels_us,
        summary_us,
        sql_rows,
        label_rows,
        returned_rows,
        stage,
        succeeded = result.is_ok(),
        "BeadsCrateAdapter::list_issues timing",
    );
    result.map_err(Into::into)
}

fn order_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<OrderKey>> {
    use rusqlite::types::ValueRef;
    let created = match row.get_ref(9)? {
        ValueRef::Null => None,
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) => Some(s.to_owned()),
            Err(_) => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(OrderKey {
        priority: row.get(8)?,
        created,
        rowid: row.get(7)?,
    }))
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueSummary> {
    let id: String = row.get(0)?;
    // Decode in the same order as upstream for rows with multiple bad fields.
    let title = row.get(1)?;
    let description = row.get::<_, Option<String>>(2)?.filter(|s| !s.is_empty());
    let status = row
        .get::<_, Option<String>>(3)?
        .map_or_else(Status::default, |s| {
            Status::from_str(&s).unwrap_or(Status::Custom(s))
        });
    let priority = row.get::<_, Option<i32>>(4)?.unwrap_or(2);
    let issue_type = row
        .get::<_, Option<String>>(5)?
        .and_then(|s| IssueType::from_str(&s).ok())
        .unwrap_or_default();
    let assignee = row.get::<_, Option<String>>(6)?.filter(|s| !s.is_empty());
    Ok(IssueSummary {
        url: format!("beads://{id}"),
        id,
        source: PmSource::Beads,
        title,
        description,
        status: status.to_string(),
        priority: Some(priority),
        issue_type: Some(issue_type.to_string()),
        assignee,
        labels: Vec::new(),
    })
}

fn legacy_column_error(error: rusqlite::Error) -> rusqlite::Error {
    // Preserve error text and typed causes despite moving the selected columns.
    const ORIGINAL: [usize; 7] = [0, 2, 3, 7, 8, 9, 10];
    match error {
        rusqlite::Error::InvalidColumnType(i, name, ty) => {
            rusqlite::Error::InvalidColumnType(ORIGINAL[i], name, ty)
        }
        rusqlite::Error::FromSqlConversionFailure(i, ty, cause) => {
            rusqlite::Error::FromSqlConversionFailure(ORIGINAL[i], ty, cause)
        }
        rusqlite::Error::IntegralValueOutOfRange(i, value) => {
            rusqlite::Error::IntegralValueOutOfRange(ORIGINAL[i], value)
        }
        other => other,
    }
}

fn labels_for_issues(
    conn: &Connection,
    ids: &[String],
    work: &mut QueryWork,
) -> rusqlite::Result<HashMap<String, Vec<String>>> {
    // Match pinned SqliteStorage::get_labels_for_issues, including batch size,
    // label ordering, and loading before adapter pagination.
    const SQLITE_VAR_LIMIT: usize = 900;
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    for chunk in ids.chunks(SQLITE_VAR_LIMIT) {
        let sql = format!(
            "SELECT issue_id, label FROM labels WHERE issue_id IN ({}) ORDER BY issue_id, label",
            vec!["?"; chunk.len()].join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            work.labels += 1;
            let (id, label) = row?;
            labels.entry(id).or_default().push(label);
        }
    }
    Ok(labels)
}

#[cfg(test)]
mod tests;
