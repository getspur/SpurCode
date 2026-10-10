//! `IssueTracker` trait impl for `BeadsCrateAdapter`.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use async_trait::async_trait;
use beads_rust::model::EventType;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, ToSql, Transaction};

use crate::adapter::IssueTracker;
use crate::beads_crate::adapter::BeadsCrateAdapter;
use crate::poll_cursor::{PollCursor, POLL_FETCH_LIMIT};
use crate::types::{
    AtomicUpdateOutcome, AtomicUpdatePrecondition, Issue, IssueCreate, IssueFilter, IssueSummary,
    IssueUpdate, PmEvent, PmSource,
};

fn validate_create_label(label: &str) -> anyhow::Result<()> {
    beads_rust::validation::LabelValidator::validate(label)
        .map_err(|error| anyhow::anyhow!("Validation failed: {error}"))
}

fn validate_create_labels<'a>(labels: impl IntoIterator<Item = &'a String>) -> anyhow::Result<()> {
    for label in labels {
        validate_create_label(label)?;
    }
    Ok(())
}

fn validate_added_label(label: &str) -> anyhow::Result<()> {
    if label.is_empty() {
        anyhow::bail!("Validation failed: label: cannot be empty");
    }
    if !label
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
    {
        anyhow::bail!(
            "Validation failed: label: invalid characters (only alphanumeric, hyphen, underscore, colon allowed)"
        );
    }
    Ok(())
}

fn validate_added_labels<'a>(labels: impl IntoIterator<Item = &'a String>) -> anyhow::Result<()> {
    for label in labels {
        validate_added_label(label)?;
    }
    Ok(())
}

const ATOMIC_UPDATE_KEY_PREFIX: &str = "spur:atomic-issue-update:";

fn apply_field_update_in_transaction(
    tx: &Transaction<'_>,
    context: &mut beads_rust::storage::sqlite::MutationContext,
    issue: &mut beads_rust::model::Issue,
    issue_id: &str,
    update: &IssueUpdate,
) -> beads_rust::error::Result<bool> {
    let has_field_update = update.status.is_some()
        || update.priority.is_some()
        || update.assignee.is_some()
        || update.body.is_some()
        || update.source_system.is_some()
        || update.source_repo.is_some()
        || update.external_ref.is_some();
    if !has_field_update {
        return Ok(false);
    }

    let mut set_clauses: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    let mut add_update = |field: &str, value: Box<dyn ToSql>| {
        set_clauses.push(format!("{field} = ?"));
        values.push(value);
    };

    if let Some(status) = update.status.as_deref() {
        let old_status = issue.status.as_str().to_string();
        let parsed =
            beads_rust::model::Status::from_str(status).unwrap_or(beads_rust::model::Status::Open);
        issue.status = parsed.clone();
        add_update("status", Box::new(parsed.as_str().to_string()));
        context.record_field_change(
            EventType::StatusChanged,
            issue_id,
            Some(old_status),
            Some(parsed.as_str().to_string()),
            None,
        );

        if parsed == beads_rust::model::Status::Closed {
            context.record_event(EventType::Closed, issue_id, None);
            if issue.closed_at.is_none() {
                issue.closed_at = Some(Utc::now());
            }
        } else {
            issue.closed_at = None;
        }
        add_update(
            "closed_at",
            Box::new(issue.closed_at.map(|closed_at| closed_at.to_rfc3339())),
        );
        context.invalidate_cache();
    }

    if let Some(priority) = update.priority {
        let old_priority = issue.priority.0;
        issue.priority = beads_rust::model::Priority(priority);
        add_update("priority", Box::new(priority));
        if priority != old_priority {
            context.record_field_change(
                EventType::PriorityChanged,
                issue_id,
                Some(old_priority.to_string()),
                Some(priority.to_string()),
                None,
            );
        }
    }

    if let Some(assignee) = update.assignee.as_ref() {
        let old_assignee = issue.assignee.clone();
        let new_assignee = (!assignee.is_empty()).then(|| assignee.clone());
        issue.assignee.clone_from(&new_assignee);
        add_update("assignee", Box::new(new_assignee.clone()));
        if old_assignee != new_assignee {
            context.record_field_change(
                EventType::AssigneeChanged,
                issue_id,
                old_assignee,
                new_assignee,
                None,
            );
        }
    }

    if let Some(body) = update.body.as_ref() {
        issue.description = Some(body.clone());
        add_update("description", Box::new(body.clone()));
    }

    if let Some(source_system) = update.source_system.as_ref() {
        issue.source_system.clone_from(source_system);
        add_update("source_system", Box::new(source_system.clone()));
    }

    if let Some(source_repo) = update.source_repo.as_ref() {
        issue.source_repo.clone_from(source_repo);
        add_update("source_repo", Box::new(source_repo.clone()));
    }

    if let Some(external_ref) = update.external_ref.as_ref() {
        issue.external_ref.clone_from(external_ref);
        add_update("external_ref", Box::new(external_ref.clone()));
    }

    let now = Utc::now();
    issue.updated_at = now;
    add_update("updated_at", Box::new(now.to_rfc3339()));
    add_update("content_hash", Box::new(issue.compute_content_hash()));
    let sql = format!("UPDATE issues SET {} WHERE id = ?", set_clauses.join(", "));
    values.push(Box::new(issue_id.to_string()));
    let value_refs: Vec<&dyn ToSql> = values.iter().map(AsRef::as_ref).collect();
    tx.execute(&sql, value_refs.as_slice())?;
    context.mark_dirty(issue_id);
    Ok(true)
}

fn apply_label_comment_update_in_transaction(
    tx: &Transaction<'_>,
    context: &mut beads_rust::storage::sqlite::MutationContext,
    issue_id: &str,
    actor: &str,
    add_labels: &[String],
    remove_labels: &[String],
    comment: Option<&str>,
) -> beads_rust::error::Result<bool> {
    let mut changed = false;

    for label in add_labels {
        let rows = tx.execute(
            "INSERT OR IGNORE INTO labels (issue_id, label) VALUES (?, ?)",
            params![issue_id, label],
        )?;
        if rows > 0 {
            context.record_event(
                EventType::LabelAdded,
                issue_id,
                Some(format!("Added label {label}")),
            );
            changed = true;
        }
    }

    for label in remove_labels {
        let rows = tx.execute(
            "DELETE FROM labels WHERE issue_id = ? AND label = ?",
            params![issue_id, label],
        )?;
        if rows > 0 {
            context.record_event(
                EventType::LabelRemoved,
                issue_id,
                Some(format!("Removed label {label}")),
            );
            changed = true;
        }
    }

    if let Some(comment) = comment {
        tx.execute(
            "INSERT INTO comments (issue_id, author, text, created_at)
             VALUES (?, ?, ?, CURRENT_TIMESTAMP)",
            params![issue_id, actor, comment],
        )?;
        context.record_event(EventType::Commented, issue_id, Some(comment.to_string()));
        changed = true;
    }

    if changed {
        tx.execute(
            "UPDATE issues SET updated_at = ? WHERE id = ?",
            params![Utc::now().to_rfc3339(), issue_id],
        )?;
        context.mark_dirty(issue_id);
    }

    Ok(changed)
}

fn apply_issue_updates_transaction(
    storage: &mut beads_rust::storage::sqlite::SqliteStorage,
    actor: &str,
    idempotency_key: Option<&str>,
    preconditions: &[AtomicUpdatePrecondition],
    updates: &[(String, IssueUpdate)],
) -> anyhow::Result<AtomicUpdateOutcome> {
    if matches!(idempotency_key, Some("")) {
        anyhow::bail!("atomic update idempotency key cannot be empty");
    }
    for (_, update) in updates {
        validate_added_labels(&update.add_labels)?;
    }
    if idempotency_key.is_some() {
        let update_issue_ids: HashSet<&str> = updates
            .iter()
            .map(|(issue_id, _)| issue_id.as_str())
            .collect();
        let precondition_issue_ids: HashSet<&str> = preconditions
            .iter()
            .map(|precondition| precondition.issue_id.as_str())
            .collect();
        anyhow::ensure!(
            precondition_issue_ids.len() == preconditions.len(),
            "atomic update preconditions contain duplicate issue IDs"
        );
        anyhow::ensure!(
            update_issue_ids == precondition_issue_ids,
            "atomic update requires exactly one precondition per updated issue"
        );
    }

    // The adapter write lock is already held while this function runs. Load
    // the hash inputs before opening the IMMEDIATE transaction, then keep this
    // owned snapshot in step with every ordered field update below.
    let mut issues = HashMap::new();
    for (issue_id, _) in updates {
        if issues.contains_key(issue_id) {
            continue;
        }
        let issue = storage
            .get_issue(issue_id)?
            .ok_or_else(|| anyhow::anyhow!("issue not found: {issue_id}"))?;
        issues.insert(issue_id.clone(), issue);
    }

    let marker_key = idempotency_key.map(|key| format!("{ATOMIC_UPDATE_KEY_PREFIX}{key}"));
    // Preconditions describe the caller's observation, not the logical
    // command identity. A later retry may observe post-commit counts and must
    // still recognize the already-applied update payload.
    let marker_value = serde_json::to_string(updates)?;
    let outcome = storage.mutate("update_issues_atomically", actor, |tx, context| {
        if let Some(marker_key) = marker_key.as_deref() {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT value FROM metadata WHERE key = ?",
                    [marker_key],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing) = existing {
                if existing != marker_value {
                    return Err(beads_rust::error::BeadsError::validation(
                        "idempotency_key",
                        "atomic update key was reused with a different payload",
                    ));
                }
                return Ok(AtomicUpdateOutcome::AlreadyApplied);
            }
        }

        for precondition in preconditions {
            let actual: i64 = tx.query_row(
                "SELECT COUNT(*) FROM comments WHERE issue_id = ?",
                [&precondition.issue_id],
                |row| row.get(0),
            )?;
            if actual as u64 != precondition.expected_comment_count {
                return Err(beads_rust::error::BeadsError::validation(
                    "preconditions",
                    format!(
                        "issue '{}' comment count changed (expected {}, found {})",
                        precondition.issue_id, precondition.expected_comment_count, actual
                    ),
                ));
            }
        }

        if let Some(marker_key) = marker_key.as_deref() {
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES (?, ?)",
                params![marker_key, marker_value],
            )?;
        }

        for (issue_id, update) in updates {
            let issue = issues
                .get_mut(issue_id)
                .expect("all atomic-update issues were loaded during preflight");
            apply_field_update_in_transaction(tx, context, issue, issue_id, update)?;
            apply_label_comment_update_in_transaction(
                tx,
                context,
                issue_id,
                actor,
                &update.add_labels,
                &update.remove_labels,
                update.comment.as_deref(),
            )?;
        }

        Ok(AtomicUpdateOutcome::Applied)
    })?;
    Ok(outcome)
}

const CREATE_ISSUE_MAX_ATTEMPTS: usize = 5;

fn generate_checked_issue_id(
    s: &beads_rust::storage::sqlite::SqliteStorage,
    params: &IssueCreate,
    now: DateTime<Utc>,
) -> String {
    let id_gen = beads_rust::util::id::IdGenerator::with_defaults();
    let issue_count = s.count_issues().unwrap_or(0).saturating_add(1);
    id_gen.generate(
        &params.title,
        params.description.as_deref(),
        Some("spur"),
        now,
        issue_count,
        |candidate| s.id_exists(candidate).unwrap_or(false),
    )
}

fn build_br_issue(
    params: &IssueCreate,
    actor: &str,
    id: String,
    now: DateTime<Utc>,
) -> beads_rust::model::Issue {
    let issue_type = params
        .issue_type
        .as_deref()
        .map(|t| {
            beads_rust::model::IssueType::from_str(t).unwrap_or(beads_rust::model::IssueType::Task)
        })
        .unwrap_or_default();

    let priority = params
        .priority
        .map(beads_rust::model::Priority)
        .unwrap_or_default();

    beads_rust::model::Issue {
        id,
        title: params.title.clone(),
        description: params.description.clone(),
        status: beads_rust::model::Status::Open,
        priority,
        issue_type,
        created_at: now,
        updated_at: now,
        assignee: params.assignee.clone(),
        owner: None,
        estimated_minutes: params.estimate_minutes.and_then(|m| i32::try_from(m).ok()),
        due_at: None,
        defer_until: None,
        external_ref: params.external_ref.clone(),
        ephemeral: false,
        content_hash: None,
        design: None,
        acceptance_criteria: None,
        notes: None,
        created_by: Some(actor.to_string()),
        closed_at: None,
        close_reason: None,
        closed_by_session: None,
        source_system: params.source_system.clone(),
        source_repo: params.source_repo.clone(),
        deleted_at: None,
        deleted_by: None,
        delete_reason: None,
        original_type: None,
        compaction_level: None,
        compacted_at: None,
        compacted_at_commit: None,
        original_size: None,
        sender: None,
        pinned: false,
        is_template: false,
        labels: Vec::new(),
        dependencies: Vec::new(),
        comments: Vec::new(),
    }
}

fn create_issue_with_retry<F>(
    s: &mut beads_rust::storage::sqlite::SqliteStorage,
    params: IssueCreate,
    actor: &str,
    mut before_create: F,
) -> anyhow::Result<String>
where
    F: FnMut(
        &mut beads_rust::storage::sqlite::SqliteStorage,
        &beads_rust::model::Issue,
    ) -> anyhow::Result<()>,
{
    for attempt in 0..CREATE_ISSUE_MAX_ATTEMPTS {
        let now = Utc::now();
        let id = generate_checked_issue_id(s, &params, now);
        let issue = build_br_issue(&params, actor, id.clone(), now);

        before_create(s, &issue)?;

        if let Err(error) = s.create_issue(&issue, actor) {
            let error = anyhow::Error::from(error);
            if attempt + 1 < CREATE_ISSUE_MAX_ATTEMPTS && is_retryable_issue_id_collision(&error) {
                continue;
            }
            return Err(error);
        }

        if !params.labels.is_empty() {
            s.set_labels(&id, &params.labels, actor)?;
        }

        if let Some(parent) = params.parent.as_deref() {
            s.add_dependency(&id, parent, "parent-child", actor)?;
        }
        for dep in &params.depends_on {
            s.add_dependency(&id, dep, "blocks", actor)?;
        }

        return Ok(id);
    }

    unreachable!("create issue retry loop always returns on final attempt")
}

fn is_retryable_issue_id_collision(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<beads_rust::error::BeadsError>()
            .is_some_and(|error| matches!(error, beads_rust::error::BeadsError::IdCollision { .. }))
            || cause
                .to_string()
                .contains("UNIQUE constraint failed: issues.id")
    })
}

pub(crate) fn br_to_pm_issue(br: beads_rust::model::Issue, blocked_by: Vec<String>) -> Issue {
    let url = format!("beads://{}", br.id);
    Issue {
        id: br.id,
        source: PmSource::Beads,
        title: br.title,
        body: br.description.unwrap_or_default(),
        status: br.status.to_string(),
        labels: br.labels,
        assignee: br.assignee,
        url,
        priority: Some(br.priority.0),
        issue_type: Some(br.issue_type.to_string()),
        blocked_by,
        due_at: br.due_at,
        source_system: br.source_system,
        source_repo: br.source_repo,
        external_ref: br.external_ref,
        created_at: br.created_at,
        updated_at: br.updated_at,
    }
}

pub(crate) fn br_to_pm_summary(br: beads_rust::model::Issue) -> IssueSummary {
    let url = format!("beads://{}", br.id);
    IssueSummary {
        id: br.id,
        source: PmSource::Beads,
        title: br.title,
        status: br.status.to_string(),
        labels: br.labels,
        url,
        priority: Some(br.priority.0),
        issue_type: Some(br.issue_type.to_string()),
        assignee: br.assignee,
        description: br.description,
    }
}

/// Version 1 query-shape bits, least significant first: status, type, labels,
/// assignee, priority min/max, text, since, limit, nonzero offset, include closed.
/// This classifies query structure, not identity: never hash or log filter values.
fn list_query_shape(filter: &IssueFilter) -> u16 {
    [
        filter.status.is_some(),
        filter.issue_type.is_some(),
        !filter.labels.is_empty(),
        filter.assignee.is_some(),
        filter.priority_min.is_some(),
        filter.priority_max.is_some(),
        filter.text_search.is_some(),
        filter.since.is_some(),
        filter.limit.is_some(),
        filter.offset.unwrap_or(0) != 0,
        filter.include_closed,
    ]
    .into_iter()
    .enumerate()
    .fold(0, |shape, (bit, present)| {
        shape | (u16::from(present) << bit)
    })
}

impl BeadsCrateAdapter {
    pub(crate) async fn poll_with_limit(&self, limit: usize) -> anyhow::Result<Vec<PmEvent>> {
        let _cursor_trace = crate::lock_trace::LockTraceGuard::lock(
            "beads.cursor",
            "BeadsCrateAdapter::poll_with_limit",
        );
        let mut guard = self.cursor.lock().await;
        let prior_cursor = guard.clone();
        let had_prior = prior_cursor.is_some();

        // Mirror BeadsAdapter::poll_with_limit: pull bounded open set,
        // apply the boundary-safe predicate client-side, advance cursor.
        let summaries = self
            .list_issues(IssueFilter {
                status: Some("open".to_string()),
                limit: Some(limit),
                ..Default::default()
            })
            .await?;

        let saturated = summaries.len() == limit;

        let mut kept: Vec<Issue> = Vec::with_capacity(summaries.len());
        for sum in summaries {
            let issue = self.get_issue(&sum.id).await?;
            let pass = match prior_cursor.as_ref() {
                None => true,
                Some(c) => c.allows(&issue.id, issue.updated_at),
            };
            if pass {
                kept.push(issue);
            }
        }

        let new_cursor: Option<PollCursor> = if !kept.is_empty() {
            if saturated {
                tracing::warn!(
                    limit,
                    kept_count = kept.len(),
                    "BeadsCrateAdapter::poll fetch saturated; preserving cursor"
                );
                prior_cursor.clone()
            } else {
                let max_ts = kept.iter().map(|i| i.updated_at).max().unwrap();
                let ids_at_max: HashSet<String> = kept
                    .iter()
                    .filter(|i| i.updated_at == max_ts)
                    .map(|i| i.id.clone())
                    .collect();
                Some(PollCursor {
                    ts: max_ts,
                    ids_at_boundary: ids_at_max,
                })
            }
        } else if let Some(existing) = prior_cursor.clone() {
            Some(existing)
        } else {
            Some(PollCursor {
                ts: Utc::now(),
                ids_at_boundary: HashSet::new(),
            })
        };

        let events: Vec<PmEvent> = kept
            .into_iter()
            .map(|issue| {
                let summary = IssueSummary {
                    id: issue.id.clone(),
                    source: PmSource::Beads,
                    title: issue.title,
                    status: issue.status,
                    labels: issue.labels,
                    url: issue.url,
                    priority: issue.priority,
                    issue_type: issue.issue_type,
                    assignee: issue.assignee,
                    description: Some(issue.body).filter(|b| !b.trim().is_empty()),
                };
                if had_prior {
                    PmEvent::IssueUpdated(summary)
                } else {
                    PmEvent::IssueCreated(summary)
                }
            })
            .collect();

        let cursor_to_persist = new_cursor.clone();
        *guard = new_cursor;
        drop(guard);

        if let (Some(path), Some(cursor)) =
            (self.config.cursor_path.as_ref(), cursor_to_persist.as_ref())
        {
            if let Err(e) = cursor.write_to(path) {
                tracing::warn!(?path, "failed to write cursor file: {e}");
            }
        }

        Ok(events)
    }
}

#[async_trait]
impl IssueTracker for BeadsCrateAdapter {
    async fn get_issue(&self, id: &str) -> anyhow::Result<Issue> {
        let id = id.to_string();
        self.read(move |s| {
            // PROBE: issue_detail_latency — split the SQLite queries so we know
            // whether the cost is in the issues row read or the label join.
            let q1_started = std::time::Instant::now();
            let mut br = s
                .get_issue(&id)?
                .ok_or_else(|| anyhow::anyhow!("issue {id} not found"))?;
            let q1_ms = q1_started.elapsed().as_millis() as u64;
            // `get_issue` only reads the `issues` table; labels are stored
            // out-of-line and must be loaded separately.
            let q2_started = std::time::Instant::now();
            let mut by_id = s.get_labels_for_issues(std::slice::from_ref(&id))?;
            let q2_ms = q2_started.elapsed().as_millis() as u64;
            br.labels = by_id.remove(&id).unwrap_or_default();
            tracing::info!(
                target: "issue_probe",
                site = "beads_get_issue_queries",
                id = %id,
                q1_get_issue_ms = q1_ms,
                q2_get_labels_ms = q2_ms,
                n_labels = br.labels.len(),
                "per-query timings inside get_issue closure",
            );
            let blocked_by = s.get_blockers(&id)?;
            Ok(br_to_pm_issue(br, blocked_by))
        })
        .await
    }

    #[tracing::instrument(
        name = "beads_list_issues",
        target = "issue_probe",
        level = "info",
        skip_all,
        fields(
            operation = "list_issues",
            query_shape = %format_args!("list_v1:{:03x}", list_query_shape(&filter)),
            status_filter = filter.status.is_some(),
            type_filter = filter.issue_type.is_some(),
            label_filter_count = filter.labels.len(),
            assignee_filter = filter.assignee.is_some(),
            priority_min = ?filter.priority_min,
            priority_max = ?filter.priority_max,
            text_filter = filter.text_search.is_some(),
            since_filter = filter.since.is_some(),
            limit = ?filter.limit,
            offset = filter.offset.unwrap_or(0),
            include_closed = filter.include_closed || filter.status.is_some(),
        )
    )]
    async fn list_issues(&self, filter: IssueFilter) -> anyhow::Result<Vec<IssueSummary>> {
        self.read_summaries(filter).await
    }

    async fn create_issue(&self, params: IssueCreate) -> anyhow::Result<String> {
        let actor = self.actor();
        self.write(move |s| {
            validate_create_labels(&params.labels)?;
            if let Some(external_ref) = params.external_ref.as_deref() {
                if let Some(existing) = s.find_by_external_ref(external_ref)? {
                    return Ok(existing.id);
                }
            }

            create_issue_with_retry(s, params, &actor, |_, _| Ok(()))
        })
        .await
    }

    async fn find_by_external_ref(&self, external_ref: &str) -> anyhow::Result<Option<Issue>> {
        let external_ref = external_ref.to_string();
        self.read(move |s| {
            let Some(issue) = s.find_by_external_ref(&external_ref)? else {
                return Ok(None);
            };
            let blocked_by = s.get_blockers(&issue.id)?;
            Ok(Some(br_to_pm_issue(issue, blocked_by)))
        })
        .await
    }

    async fn update_issue(&self, id: &str, update: IssueUpdate) -> anyhow::Result<()> {
        let id = id.to_string();
        let actor = self.actor();
        self.write(move |s| {
            apply_issue_updates_transaction(s, &actor, None, &[], &[(id, update)])?;
            Ok(())
        })
        .await
    }

    async fn update_issues_atomically(
        &self,
        idempotency_key: &str,
        preconditions: Vec<AtomicUpdatePrecondition>,
        updates: Vec<(String, IssueUpdate)>,
    ) -> anyhow::Result<AtomicUpdateOutcome> {
        let idempotency_key = idempotency_key.to_string();
        let actor = self.actor();
        self.write(move |storage| {
            apply_issue_updates_transaction(
                storage,
                &actor,
                Some(&idempotency_key),
                &preconditions,
                &updates,
            )
        })
        .await
    }

    async fn add_dependency(&self, issue_id: &str, depends_on_id: &str) -> anyhow::Result<()> {
        let issue_id = issue_id.to_string();
        let depends_on_id = depends_on_id.to_string();
        let actor = self.actor();
        self.write(move |s| {
            s.add_dependency(&issue_id, &depends_on_id, "blocks", &actor)?;
            Ok(())
        })
        .await
    }

    async fn poll(&self) -> anyhow::Result<Vec<PmEvent>> {
        self.poll_with_limit(POLL_FETCH_LIMIT).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use beads_rust::model::{EventType, Issue as BrIssue, IssueType, Priority, Status};
    use chrono::{TimeZone, Timelike, Utc};
    use tempfile::TempDir;

    use super::{create_issue_with_retry, generate_checked_issue_id, ATOMIC_UPDATE_KEY_PREFIX};
    use crate::adapter::IssueTracker;
    use crate::beads_crate::adapter::{AdapterConfig, BeadsCrateAdapter};
    use crate::types::{
        AtomicUpdateOutcome, AtomicUpdatePrecondition, IssueCreate, IssueFilter, IssueUpdate,
        PmEvent,
    };

    fn minimal_issue(id: &str, title: &str) -> BrIssue {
        let now = Utc::now();
        BrIssue {
            id: id.into(),
            title: title.into(),
            description: None,
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: now,
            updated_at: now,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            ephemeral: false,
            content_hash: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            source_system: None,
            source_repo: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            pinned: false,
            is_template: false,
            labels: Vec::new(),
            dependencies: Vec::new(),
            comments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn list_observability_keeps_ancestry_shape_and_stage_work() {
        use crate::beads_crate::beads_db::trace_tests::Capture;
        use tracing::instrument::WithSubscriber;
        use tracing::Instrument;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        adapter
            .batch(|s| {
                let mut issue = minimal_issue("bd-observe", "private-title");
                issue.description = Some("private-body".into());
                s.create_issue(&issue, "test")?;
                s.add_label(&issue.id, "private-label", "test")?;
                Ok(())
            })
            .await
            .unwrap();
        let capture = Capture::default();
        let dispatch = capture.dispatch();
        let caller = tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!("hygiene", caller = "reconciler")
        });
        let rows = adapter
            .list_issues(IssueFilter {
                labels: vec!["private-label".into()],
                text_search: Some("private-title".into()),
                limit: Some(1),
                ..Default::default()
            })
            .instrument(caller)
            .with_subscriber(dispatch)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].description.as_deref(), Some("private-body"));
        assert_eq!(rows[0].labels, ["private-label"]);

        let events = capture.events();
        let timing = events
            .iter()
            .find(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\""))
            .expect("list query must emit its work and timings");
        assert_eq!(
            timing
                .scope
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["hygiene", "beads_list_issues"]
        );
        assert_eq!(timing.scope[0].1 .0["caller"], "\"reconciler\"");
        let shape = &timing.scope[1].1 .0;
        assert_eq!(shape["operation"], "\"list_issues\"");
        assert!(shape.contains_key("query_shape"));
        assert_eq!(shape["label_filter_count"], "1");
        assert_eq!(shape["text_filter"], "true");
        for field in ["sql_decode_us", "labels_us", "summary_us"] {
            assert!(
                timing.fields.0[field].parse::<u64>().is_ok(),
                "missing duration: {field}"
            );
        }
        for field in ["sql_rows", "label_rows", "returned_rows"] {
            assert_eq!(timing.fields.0[field], "1", "wrong work count: {field}");
        }
        assert_eq!(timing.fields.0["succeeded"], "true");
        let read = events
            .iter()
            .find(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_read\""))
            .expect("existing adapter read timing remains available");
        assert_eq!(read.scope[0].0, "hygiene");
        assert_eq!(read.scope[1].0, "beads_list_issues");
        assert!(read.fields.0["actor_queue_us"].parse::<u64>().is_ok());
        assert!(read.fields.0["closure_us"].parse::<u64>().is_ok());
        let captured = format!("{events:?}");
        for private in ["private-title", "private-body", "private-label"] {
            assert!(
                !captured.contains(private),
                "private data leaked: {private}"
            );
        }
    }

    #[tokio::test]
    async fn list_observability_without_parent_has_stable_value_free_shape() {
        use crate::beads_crate::beads_db::trace_tests::Capture;
        use tracing::instrument::WithSubscriber;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let capture = Capture::default();
        let dispatch = capture.dispatch();
        for value in ["private-one", "private-two"] {
            adapter
                .list_issues(IssueFilter {
                    labels: vec![value.into()],
                    text_search: Some(value.into()),
                    assignee: Some(value.into()),
                    ..Default::default()
                })
                .with_subscriber(dispatch.clone())
                .await
                .unwrap();
        }
        adapter
            .list_issues(IssueFilter::default())
            .with_subscriber(dispatch)
            .await
            .unwrap();
        let events = capture.events();
        let lists: Vec<_> = events
            .iter()
            .filter(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\""))
            .collect();
        assert_eq!(
            lists.len(),
            3,
            "every list needs a stable operation label without a parent"
        );
        for event in &lists {
            assert_eq!(event.scope.len(), 1);
            assert_eq!(event.scope[0].0, "beads_list_issues");
            assert_eq!(event.scope[0].1 .0["operation"], "\"list_issues\"");
            assert_eq!(event.fields.0["returned_rows"], "0");
        }
        assert_eq!(
            lists[0].scope[0].1 .0["query_shape"],
            lists[1].scope[0].1 .0["query_shape"]
        );
        assert_ne!(
            lists[0].scope[0].1 .0["query_shape"],
            lists[2].scope[0].1 .0["query_shape"]
        );
        let captured = format!("{events:?}");
        assert!(!captured.contains("private-one"));
        assert!(!captured.contains("private-two"));
    }

    #[tokio::test]
    async fn list_observability_reports_sql_error_without_exposing_error_text() {
        use crate::beads_crate::beads_db::trace_tests::Capture;
        use tracing::instrument::WithSubscriber;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        adapter
            .batch(|s| {
                s.create_issue(&minimal_issue("bd-bad-row", "private-title"), "test")?;
                Ok(())
            })
            .await
            .unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("beads.db")).unwrap();
        conn.execute("UPDATE issues SET title = x'ff'", []).unwrap();
        let expected = adapter
            .read(|s| Ok(s.list_issues(&Default::default())?))
            .await
            .unwrap_err();
        let capture = Capture::default();
        let error = adapter
            .list_issues(IssueFilter::default())
            .with_subscriber(capture.dispatch())
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            expected.to_string(),
            "instrumentation must preserve the SQLite error"
        );
        let events = capture.events();
        let timing = events
            .iter()
            .find(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\""))
            .expect("failed list queries must retain timing and operation attribution");
        assert_eq!(timing.fields.0["succeeded"], "false");
        assert_eq!(timing.fields.0["stage"], "\"sql\"");
        assert!(timing.fields.0["sql_decode_us"].parse::<u64>().is_ok());
        assert!(!format!("{events:?}").contains("private-title"));
    }

    #[tokio::test]
    async fn list_observability_reports_label_failure_after_sql_work() {
        use crate::beads_crate::beads_db::trace_tests::Capture;
        use tracing::instrument::WithSubscriber;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        adapter
            .batch(|s| {
                s.create_issue(&minimal_issue("bd-label-error", "private-title"), "test")?;
                Ok(())
            })
            .await
            .unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("beads.db")).unwrap();
        conn.execute("DROP TABLE labels", []).unwrap();
        let expected = adapter
            .read(|s| Ok(s.get_labels_for_issues(&["bd-label-error".to_owned()])?))
            .await
            .unwrap_err();
        let capture = Capture::default();
        let error = adapter
            .list_issues(IssueFilter::default())
            .with_subscriber(capture.dispatch())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected.to_string());
        let events = capture.events();
        let timing = events
            .iter()
            .find(|e| e.fields.0.get("site").map(String::as_str) == Some("\"beads_list_issues\""))
            .expect("label errors must retain completed SQL work");
        assert_eq!(timing.fields.0["stage"], "\"labels\"");
        assert_eq!(timing.fields.0["succeeded"], "false");
        assert_eq!(timing.fields.0["sql_rows"], "1");
        assert_eq!(timing.fields.0["returned_rows"], "0");
        assert!(timing.fields.0["labels_us"].parse::<u64>().is_ok());
    }

    fn event_ids(events: &[PmEvent]) -> HashSet<String> {
        events
            .iter()
            .map(|event| match event {
                PmEvent::IssueCreated(summary) | PmEvent::IssueUpdated(summary) => {
                    summary.id.clone()
                }
            })
            .collect()
    }

    fn issue_event_is_updated(event: &PmEvent) -> bool {
        matches!(event, PmEvent::IssueUpdated(_))
    }

    fn issue_event_is_created(event: &PmEvent) -> bool {
        matches!(event, PmEvent::IssueCreated(_))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_issue_returns_seeded_issue() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let seed_issue = minimal_issue("bd-test-get-issue", "seeded issue");
        let expected_id = seed_issue.id.clone();
        let expected_title = seed_issue.title.clone();

        adapter
            .write(move |s| s.create_issue(&seed_issue, "test").map_err(Into::into))
            .await
            .unwrap();

        let issue = adapter.get_issue(&expected_id).await.unwrap();

        assert_eq!(issue.id, expected_id);
        assert_eq!(issue.title, expected_title);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_issue_round_trips_via_get_issue() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "Hello".into(),
                description: Some("Body".into()),
                priority: Some(1),
                labels: vec!["lbl-a".into(), "lbl-b".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!id.is_empty());

        let fetched = adapter.get_issue(&id).await.unwrap();
        assert_eq!(fetched.title, "Hello");
        assert_eq!(fetched.body, "Body");
        assert_eq!(fetched.priority, Some(1));
        assert_eq!(
            fetched.labels,
            vec!["lbl-a".to_string(), "lbl-b".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_issue_uses_checked_id_lookup() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let now = Utc
            .with_ymd_and_hms(2026, 5, 18, 12, 0, 0)
            .unwrap()
            .with_nanosecond(123)
            .unwrap();
        let params = IssueCreate {
            title: "colliding task".into(),
            description: Some("same body".into()),
            ..Default::default()
        };
        let colliding_id = beads_rust::util::id::IdGenerator::with_defaults().generate_candidate(
            &params.title,
            params.description.as_deref(),
            Some("spur"),
            now,
            0,
            3,
        );

        adapter
            .write({
                let params = params.clone();
                move |s| {
                    let seed_issue = minimal_issue(&colliding_id, "seeded collision");
                    s.create_issue(&seed_issue, "test")?;

                    let generated = generate_checked_issue_id(s, &params, now);

                    assert_ne!(generated, colliding_id);
                    assert!(!s.id_exists(&generated)?);
                    Ok(())
                }
            })
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_issue_retries_on_id_collision_from_storage() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .write(|s| {
                let mut collided = false;
                create_issue_with_retry(
                    s,
                    IssueCreate {
                        title: "race window".into(),
                        description: Some("body".into()),
                        ..Default::default()
                    },
                    "test",
                    |s, issue| {
                        if !collided {
                            collided = true;
                            let seed_issue = minimal_issue(&issue.id, "forced collision");
                            s.create_issue(&seed_issue, "test")?;
                        }
                        Ok(())
                    },
                )
            })
            .await
            .unwrap();

        let issues = adapter
            .list_issues(IssueFilter {
                include_closed: true,
                ..Default::default()
            })
            .await
            .unwrap();
        let ids: HashSet<_> = issues.into_iter().map(|issue| issue.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_issue_round_trips_provenance_fields() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "Imported issue".into(),
                source_system: Some("github".into()),
                source_repo: Some("getspur/spur".into()),
                external_ref: Some("github/I_kwDOExample123".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        let fetched = adapter.get_issue(&id).await.unwrap();
        assert_eq!(fetched.source_system.as_deref(), Some("github"));
        assert_eq!(fetched.source_repo.as_deref(), Some("getspur/spur"));
        assert_eq!(
            fetched.external_ref.as_deref(),
            Some("github/I_kwDOExample123")
        );

        let found = adapter
            .find_by_external_ref("github/I_kwDOExample123")
            .await
            .unwrap()
            .expect("find by external ref");
        assert_eq!(found.id, id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_issue_is_idempotent_for_existing_external_ref() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let first = adapter
            .create_issue(IssueCreate {
                title: "Imported issue".into(),
                source_system: Some("github".into()),
                source_repo: Some("getspur/spur".into()),
                external_ref: Some("github/I_kwDOExample123".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        let second = adapter
            .create_issue(IssueCreate {
                title: "Imported issue renamed upstream".into(),
                source_system: Some("github".into()),
                source_repo: Some("getspur/spur".into()),
                external_ref: Some("github/I_kwDOExample123".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(second, first);

        let issues = adapter
            .list_issues(IssueFilter {
                include_closed: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].title, "Imported issue");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_issue_can_set_and_clear_external_ref() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "Local issue".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        adapter
            .update_issue(
                &id,
                IssueUpdate {
                    external_ref: Some(Some("github/I_kwDOExample123".into())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let with_ref = adapter.get_issue(&id).await.unwrap();
        assert_eq!(
            with_ref.external_ref.as_deref(),
            Some("github/I_kwDOExample123")
        );

        adapter
            .update_issue(
                &id,
                IssueUpdate {
                    external_ref: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let without_ref = adapter.get_issue(&id).await.unwrap();
        assert_eq!(without_ref.external_ref, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_issue_changes_status_and_labels() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "X".into(),
                labels: vec!["keep".into(), "drop".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        adapter
            .update_issue(
                &id,
                IssueUpdate {
                    status: Some("closed".into()),
                    priority: Some(0),
                    add_labels: vec!["new".into()],
                    remove_labels: vec!["drop".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let after = adapter.get_issue(&id).await.unwrap();
        assert_eq!(after.status, "closed");
        assert_eq!(after.priority, Some(0));
        let mut labels = after.labels;
        labels.sort();
        assert_eq!(labels, vec!["keep".to_string(), "new".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn combined_label_comment_update_rolls_back_after_comment_failure() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "Owned plan".into(),
                labels: vec!["owner:prior".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        let dirty_issue_id = id.clone();
        adapter
            .write(move |storage| {
                storage.clear_dirty_issues(&[dirty_issue_id])?;
                Ok(())
            })
            .await
            .unwrap();
        let baseline_issue_id = id.clone();
        let (updated_at_before, event_count_before) = adapter
            .read(move |storage| {
                let issue = storage
                    .get_issue(&baseline_issue_id)?
                    .expect("created issue must exist");
                let events = storage.get_events(&baseline_issue_id, 1_000)?;
                Ok((issue.updated_at, events.len()))
            })
            .await
            .unwrap();

        adapter
            .write(|storage| {
                storage.mutate("inject_comment_failure", "test", |tx, _ctx| {
                    tx.execute_batch(
                        "CREATE TRIGGER fail_comment_insert
                         BEFORE INSERT ON comments
                         BEGIN
                           SELECT RAISE(ABORT, 'injected comment failure');
                         END;",
                    )?;
                    Ok(())
                })?;
                Ok(())
            })
            .await
            .unwrap();

        let audit = "[[spur-audit v1]]\n{\"kind\":\"ownership-migration\"}";
        let error = adapter
            .update_issue(
                &id,
                IssueUpdate {
                    add_labels: vec!["owner:system-l3".into()],
                    remove_labels: vec!["owner:prior".into()],
                    comment: Some(audit.into()),
                    ..Default::default()
                },
            )
            .await
            .expect_err("injected comment failure must reject the combined update");
        assert!(error.to_string().contains("injected comment failure"));

        let issue = adapter.get_issue(&id).await.unwrap();
        let stored_issue_id = id.clone();
        let (updated_at_after, comments, events, dirty_ids) = adapter
            .read(move |storage| {
                let issue = storage
                    .get_issue(&stored_issue_id)?
                    .expect("created issue must remain after rollback");
                let comments = storage.get_comments(&stored_issue_id)?;
                let events = storage.get_events(&stored_issue_id, 1_000)?;
                let dirty_ids = storage.get_dirty_issue_ids()?;
                Ok((issue.updated_at, comments, events, dirty_ids))
            })
            .await
            .unwrap();
        assert!(issue.labels.contains(&"owner:prior".to_string()));
        assert!(!issue.labels.contains(&"owner:system-l3".to_string()));
        assert!(
            comments.iter().all(|comment| comment.body != audit),
            "failed migration must not persist its audit comment"
        );
        assert_eq!(events.len(), event_count_before);
        assert_eq!(updated_at_after, updated_at_before);
        assert!(!dirty_ids.contains(&id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn combined_status_label_comment_update_rolls_back_after_comment_failure() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let id = adapter
            .create_issue(IssueCreate {
                title: "Review target".into(),
                labels: vec!["ready-for-review".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        let dirty_issue_id = id.clone();
        adapter
            .write(move |storage| {
                storage.clear_dirty_issues(&[dirty_issue_id])?;
                storage.mutate("inject_terminal_audit_failure", "test", |tx, _ctx| {
                    tx.execute_batch(
                        "CREATE TRIGGER fail_terminal_audit_insert
                         BEFORE INSERT ON comments
                         WHEN NEW.text LIKE '[[spur-audit v1]]%'
                         BEGIN
                           SELECT RAISE(ABORT, 'injected terminal audit failure');
                         END;",
                    )?;
                    Ok(())
                })?;
                Ok(())
            })
            .await
            .unwrap();

        let baseline_issue_id = id.clone();
        let (updated_at_before, event_count_before) = adapter
            .read(move |storage| {
                let issue = storage
                    .get_issue(&baseline_issue_id)?
                    .expect("created issue must exist");
                let events = storage.get_events(&baseline_issue_id, 1_000)?;
                Ok((issue.updated_at, events.len()))
            })
            .await
            .unwrap();

        let audit = "[[spur-audit v1]]\n{\"kind\":\"approval\"}";
        let error = adapter
            .update_issue(
                &id,
                IssueUpdate {
                    status: Some("closed".into()),
                    remove_labels: vec!["ready-for-review".into()],
                    comment: Some(audit.into()),
                    ..Default::default()
                },
            )
            .await
            .expect_err("injected audit failure must reject the complete review update");
        assert!(error
            .to_string()
            .contains("injected terminal audit failure"));

        let issue = adapter.get_issue(&id).await.unwrap();
        let stored_issue_id = id.clone();
        let (updated_at_after, comments, events, dirty_ids) = adapter
            .read(move |storage| {
                let issue = storage
                    .get_issue(&stored_issue_id)?
                    .expect("created issue must remain after rollback");
                let comments = storage.get_comments(&stored_issue_id)?;
                let events = storage.get_events(&stored_issue_id, 1_000)?;
                let dirty_ids = storage.get_dirty_issue_ids()?;
                Ok((issue.updated_at, comments, events, dirty_ids))
            })
            .await
            .unwrap();

        assert_eq!(issue.status, "open", "status must roll back with audit");
        assert!(issue.labels.contains(&"ready-for-review".to_string()));
        assert!(comments.iter().all(|comment| comment.body != audit));
        assert_eq!(events.len(), event_count_before);
        assert_eq!(updated_at_after, updated_at_before);
        assert!(!dirty_ids.contains(&id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn atomic_multi_issue_update_rolls_back_late_failure_and_retries_idempotently() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        let task_id = adapter
            .create_issue(IssueCreate {
                title: "Review target".into(),
                labels: vec!["ready-for-review".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        let epic_id = adapter
            .create_issue(IssueCreate {
                title: "Plan epic".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let ids = vec![task_id.clone(), epic_id.clone()];
        adapter
            .write(move |storage| {
                storage.clear_dirty_issues(&ids)?;
                storage.mutate("inject_late_review_failure", "test", |tx, _ctx| {
                    tx.execute_batch(
                        "CREATE TRIGGER fail_transition_audit_insert
                         BEFORE INSERT ON comments
                         WHEN NEW.text = 'transition audit'
                         BEGIN
                           SELECT RAISE(ABORT, 'injected transition audit failure');
                         END;",
                    )?;
                    Ok(())
                })?;
                Ok(())
            })
            .await
            .unwrap();

        let baseline_task_id = task_id.clone();
        let task_event_cursor = adapter
            .read(move |storage| {
                Ok(storage
                    .get_events(&baseline_task_id, 1_000)?
                    .into_iter()
                    .map(|event| event.id)
                    .max()
                    .unwrap_or_default())
            })
            .await
            .unwrap();

        let updates = vec![
            (
                task_id.clone(),
                IssueUpdate {
                    status: Some("closed".into()),
                    remove_labels: vec!["ready-for-review".into()],
                    comment: Some("Brain approved".into()),
                    ..Default::default()
                },
            ),
            (
                task_id.clone(),
                IssueUpdate {
                    comment: Some("approval audit".into()),
                    ..Default::default()
                },
            ),
            (
                epic_id.clone(),
                IssueUpdate {
                    comment: Some("transition audit".into()),
                    ..Default::default()
                },
            ),
        ];
        let preconditions = vec![
            AtomicUpdatePrecondition {
                issue_id: task_id.clone(),
                expected_comment_count: 0,
            },
            AtomicUpdatePrecondition {
                issue_id: epic_id.clone(),
                expected_comment_count: 0,
            },
        ];
        let error = adapter
            .update_issues_atomically("review-plan-task-1", preconditions.clone(), updates.clone())
            .await
            .expect_err("late audit failure must roll back the whole review transaction");
        assert!(error
            .to_string()
            .contains("injected transition audit failure"));

        let task = adapter.get_issue(&task_id).await.unwrap();
        assert_eq!(task.status, "open");
        assert!(task.labels.contains(&"ready-for-review".to_string()));
        let inspect_task_id = task_id.clone();
        let inspect_epic_id = epic_id.clone();
        let (task_comments, epic_comments, dirty_ids, marker) = adapter
            .read(move |storage| {
                Ok((
                    storage.get_comments(&inspect_task_id)?,
                    storage.get_comments(&inspect_epic_id)?,
                    storage.get_dirty_issue_ids()?,
                    storage
                        .get_metadata(&format!("{ATOMIC_UPDATE_KEY_PREFIX}review-plan-task-1"))?,
                ))
            })
            .await
            .unwrap();
        assert!(task_comments.is_empty());
        assert!(epic_comments.is_empty());
        assert!(dirty_ids.is_empty());
        assert!(
            marker.is_none(),
            "rolled-back transaction cannot claim its key"
        );

        adapter
            .write(|storage| {
                storage.mutate("remove_review_failure", "test", |tx, _ctx| {
                    tx.execute_batch("DROP TRIGGER fail_transition_audit_insert")?;
                    Ok(())
                })?;
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(
            adapter
                .update_issues_atomically(
                    "review-plan-task-1",
                    preconditions.clone(),
                    updates.clone(),
                )
                .await
                .unwrap(),
            AtomicUpdateOutcome::Applied
        );
        assert_eq!(
            adapter
                .update_issues_atomically(
                    "review-plan-task-1",
                    preconditions.clone(),
                    updates.clone(),
                )
                .await
                .unwrap(),
            AtomicUpdateOutcome::AlreadyApplied
        );

        let competing_error = adapter
            .update_issues_atomically(
                "competing-review-plan-task-1",
                vec![AtomicUpdatePrecondition {
                    issue_id: task_id.clone(),
                    expected_comment_count: 0,
                }],
                vec![(
                    task_id.clone(),
                    IssueUpdate {
                        status: Some("open".into()),
                        comment: Some("rejection audit".into()),
                        ..Default::default()
                    },
                )],
            )
            .await
            .expect_err("a stale competing review must fail its compare-and-set guard");
        assert!(competing_error
            .to_string()
            .contains("comment count changed"));

        let task = adapter.get_issue(&task_id).await.unwrap();
        assert_eq!(task.status, "closed");
        assert!(!task.labels.contains(&"ready-for-review".to_string()));
        let inspect_task_id = task_id.clone();
        let inspect_epic_id = epic_id.clone();
        let (task_comments, epic_comments, mut task_event_types, hash_matches) = adapter
            .read(move |storage| {
                let stored_task = storage
                    .get_issue(&inspect_task_id)?
                    .expect("task exists after atomic review");
                let expected_hash = stored_task.compute_content_hash();
                let task_event_types: Vec<EventType> = storage
                    .get_events(&inspect_task_id, 1_000)?
                    .into_iter()
                    .filter(|event| event.id > task_event_cursor)
                    .map(|event| event.event_type)
                    .collect();
                Ok((
                    storage.get_comments(&inspect_task_id)?,
                    storage.get_comments(&inspect_epic_id)?,
                    task_event_types,
                    stored_task.content_hash.as_deref() == Some(expected_hash.as_str()),
                ))
            })
            .await
            .unwrap();
        assert_eq!(task_comments.len(), 2);
        assert_eq!(epic_comments.len(), 1);
        task_event_types.reverse();
        assert_eq!(
            task_event_types,
            vec![
                EventType::StatusChanged,
                EventType::Closed,
                EventType::LabelRemoved,
                EventType::Commented,
                EventType::Commented,
            ],
            "the atomic path must preserve the native Beads event order"
        );
        assert!(
            hash_matches,
            "atomic field updates must refresh content_hash"
        );

        drop(adapter);
        let reopened = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();
        assert_eq!(
            reopened
                .update_issues_atomically(
                    "review-plan-task-1",
                    vec![
                        AtomicUpdatePrecondition {
                            issue_id: task_id,
                            expected_comment_count: 2,
                        },
                        AtomicUpdatePrecondition {
                            issue_id: epic_id,
                            expected_comment_count: 1,
                        },
                    ],
                    updates,
                )
                .await
                .unwrap(),
            AtomicUpdateOutcome::AlreadyApplied,
            "the idempotency marker must survive adapter restart"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dependency_links_two_issues() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let parent = adapter
            .create_issue(IssueCreate {
                title: "P".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let child = adapter
            .create_issue(IssueCreate {
                title: "C".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        adapter.add_dependency(&child, &parent).await.unwrap();

        // Verify the edge through the storage's own dependency reader.
        let parent_for_check = parent.clone();
        let child_for_check = child.clone();
        let deps = adapter
            .read(move |s| Ok(s.get_dependencies(&child_for_check)?))
            .await
            .unwrap();
        assert!(
            deps.contains(&parent_for_check),
            "expected {parent_for_check} in deps, got {deps:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_emits_created_then_quiesces() {
        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        adapter
            .create_issue(IssueCreate {
                title: "X".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let first = adapter.poll().await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0], PmEvent::IssueCreated(_)));

        let second = adapter.poll().await.unwrap();
        assert!(
            second.is_empty(),
            "second poll should be empty, got {:?}",
            second
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_does_not_advance_cursor_on_limit_saturation() {
        let dir = TempDir::new().unwrap();
        let cursor_path = dir.path().join(".spur-test-cursor");
        let config = AdapterConfig {
            cursor_path: Some(cursor_path),
            ..AdapterConfig::default()
        };
        let adapter = BeadsCrateAdapter::open(dir.path(), config).await.unwrap();

        let initial = adapter
            .create_issue(IssueCreate {
                title: "Initial".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let prime = adapter.poll_with_limit(10).await.unwrap();
        assert_eq!(prime.len(), 1);
        adapter
            .update_issue(
                &initial,
                IssueUpdate {
                    status: Some("closed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let cursor_before = adapter
            .cursor
            .lock()
            .await
            .clone()
            .expect("prime poll should set cursor");

        let mut new_ids = HashSet::new();
        for i in 0..5 {
            let id = adapter
                .create_issue(IssueCreate {
                    title: format!("Issue {i}"),
                    ..Default::default()
                })
                .await
                .unwrap();
            new_ids.insert(id);
        }

        let saturated_events = adapter.poll_with_limit(3).await.unwrap();
        assert_eq!(saturated_events.len(), 3);
        assert!(saturated_events.iter().all(issue_event_is_updated));
        let cursor_after = adapter
            .cursor
            .lock()
            .await
            .clone()
            .expect("saturated poll with prior cursor should preserve it");
        assert_eq!(cursor_after.ts, cursor_before.ts);
        assert_eq!(cursor_after.ids_at_boundary, cursor_before.ids_at_boundary);

        let saturated_ids = event_ids(&saturated_events);
        for id in &saturated_ids {
            adapter
                .update_issue(
                    id,
                    IssueUpdate {
                        status: Some("closed".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }

        let remaining_events = adapter.poll_with_limit(10).await.unwrap();
        assert!(remaining_events.iter().all(issue_event_is_updated));
        let remaining_ids = event_ids(&remaining_events);
        let expected_remaining: HashSet<String> =
            new_ids.difference(&saturated_ids).cloned().collect();
        assert_eq!(remaining_ids, expected_remaining);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saturated_first_poll_keeps_cursor_unset_until_backlog_drains() {
        let dir = TempDir::new().unwrap();
        let cursor_path = dir.path().join(".spur-test-cursor");
        let config = AdapterConfig {
            cursor_path: Some(cursor_path.clone()),
            ..AdapterConfig::default()
        };
        let adapter = BeadsCrateAdapter::open(dir.path(), config).await.unwrap();

        let mut all_ids = HashSet::new();
        for i in 0..5 {
            let id = adapter
                .create_issue(IssueCreate {
                    title: format!("Issue {i}"),
                    ..Default::default()
                })
                .await
                .unwrap();
            all_ids.insert(id);
        }

        let first_events = adapter.poll_with_limit(3).await.unwrap();
        assert_eq!(first_events.len(), 3);
        assert!(first_events.iter().all(issue_event_is_created));
        assert!(
            adapter.cursor.lock().await.is_none(),
            "first saturated poll should leave in-memory cursor unset"
        );
        assert!(
            !cursor_path.exists(),
            "first saturated poll should not write a cursor file"
        );

        let first_ids = event_ids(&first_events);
        for id in &first_ids {
            adapter
                .update_issue(
                    id,
                    IssueUpdate {
                        status: Some("closed".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }

        let second_events = adapter.poll_with_limit(10).await.unwrap();
        assert!(second_events.iter().all(issue_event_is_created));
        let second_ids = event_ids(&second_events);
        let expected_second: HashSet<String> = all_ids.difference(&first_ids).cloned().collect();
        assert_eq!(second_ids, expected_second);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_issues_returns_seeded_data() {
        use crate::types::IssueFilter;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        adapter
            .batch(|s| {
                for i in 0..3 {
                    let issue = minimal_issue(&format!("bd-list-{i}"), &format!("issue {i}"));
                    s.create_issue(&issue, "test")?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let summaries = adapter.list_issues(IssueFilter::default()).await.unwrap();
        assert_eq!(summaries.len(), 3);
        let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"bd-list-0"));
        assert!(ids.contains(&"bd-list-1"));
        assert!(ids.contains(&"bd-list-2"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_issues_include_closed_excludes_tombstones() {
        use crate::types::IssueFilter;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        let open_id = adapter
            .create_issue(IssueCreate {
                title: "Open issue".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let closed_id = adapter
            .create_issue(IssueCreate {
                title: "Closed issue".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let deleted_id = adapter
            .create_issue(IssueCreate {
                title: "Deleted issue".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        adapter
            .update_issue(
                &closed_id,
                IssueUpdate {
                    status: Some("closed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let deleted_id_for_write = deleted_id.clone();
        adapter
            .write(move |s| {
                s.delete_issue(&deleted_id_for_write, "test", "deleted", None)?;
                Ok(())
            })
            .await
            .unwrap();

        let summaries = adapter
            .list_issues(IssueFilter {
                include_closed: true,
                ..Default::default()
            })
            .await
            .unwrap();

        let ids: HashSet<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(open_id.as_str()));
        assert!(ids.contains(closed_id.as_str()));
        assert!(
            !ids.contains(deleted_id.as_str()),
            "include_closed should not return tombstones"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_issues_applies_offset_after_fetching_limit_window() {
        use crate::types::IssueFilter;

        let dir = TempDir::new().unwrap();
        let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
            .await
            .unwrap();

        adapter
            .batch(|s| {
                for i in 0..5 {
                    let mut issue = minimal_issue(&format!("bd-offset-{i}"), &format!("issue {i}"));
                    issue.priority = Priority(i);
                    s.create_issue(&issue, "test")?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let summaries = adapter
            .list_issues(IssueFilter {
                limit: Some(2),
                offset: Some(2),
                include_closed: true,
                ..Default::default()
            })
            .await
            .unwrap();

        let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["bd-offset-2", "bd-offset-3"]);
    }
}
