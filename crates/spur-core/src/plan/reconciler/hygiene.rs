//! Serialize sweeps and acknowledge only their pre-work snapshot. State is
//! staged locally: errors and dropped futures preserve the old cursor and all
//! pending work. Repairs are idempotent, so partial batches can safely replay.
use super::*;
use spur_pm::advanced::HygieneCursor;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct HygieneState {
    cursor: Option<HygieneCursor>,
    // Successfully checked empty audits, retained ONLY while dispatch-grace
    // work is pending. Bound is the eligible open pending set; no TTL/cap.
    pending: BTreeMap<String, spur_pm::IssueSummary>,
}

impl Reconciler {
    pub(super) async fn run_index_hygiene_sweep(&self) -> anyhow::Result<bool> {
        crate::server::require_feature(
            spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
            self.feature_gate.as_ref(),
        )
        .map_err(|error| anyhow::anyhow!(crate::server::feature_error_message(error)))?;
        let Some(adv) = self.pm.advanced() else {
            return Ok(false);
        };
        let mut state = self.hygiene.lock().await;
        let Some(batch) = adv.hygiene_changes(state.cursor.as_ref()).await? else {
            let open = self
                .pm
                .list_issues(spur_pm::IssueFilter {
                    status: Some("open".into()),
                    ..Default::default()
                })
                .await?;
            let mut did_work = false;
            for issue in open {
                let comments = adv.list_comments(&issue.id).await?;
                let audits =
                    crate::plan::projector::collect_sorted_audits_for_issue(&issue.id, comments)?;
                did_work |= self.index_hygiene_sweep(adv, &issue, &audits, None).await?;
            }
            *state = HygieneState::default();
            return Ok(did_work);
        };
        let mut pending = if batch.reseed {
            BTreeMap::new()
        } else {
            state.pending.clone()
        };
        for id in &batch.affected_ids {
            // Includes deletion/eligibility exit, plus issue, comment and parent
            // changes that invalidate a previously checked empty-audit result.
            pending.remove(id);
        }
        let mut did_work = false;
        // Only unchanged pending candidates reuse the successful empty result.
        // Process these before adding this batch's newly checked candidates.
        for issue in pending.values().cloned().collect::<Vec<_>>() {
            if self.reconcile_label_only_dispatch(&issue).await? {
                did_work = true;
                pending.remove(&issue.id);
            }
        }
        for candidate in batch.candidates {
            let issue = candidate.issue;
            pending.remove(&issue.id);
            let comments = adv.list_comments(&issue.id).await?;
            let audits =
                crate::plan::projector::collect_sorted_audits_for_issue(&issue.id, comments)?;
            let repaired = self
                .index_hygiene_sweep(adv, &issue, &audits, Some(&candidate.parent_ids))
                .await?;
            did_work |= repaired;
            if audits.is_empty()
                && !repaired
                && issue.labels.iter().any(|label| {
                    delegation_label_value(label).is_some()
                        || crate::plan::labels::parse_lease_expires_at(label).is_some()
                })
            {
                pending.insert(issue.id.clone(), issue);
            }
        }
        // This EXACT cursor preceded comments and label writes. A newer cursor
        // could swallow concurrent comments or repairs, even if summaries were
        // already published elsewhere. Never fetch/ack a post-write cursor here.
        *state = HygieneState {
            cursor: Some(batch.cursor),
            pending,
        };
        Ok(did_work)
    }

    pub(super) async fn expected_plan_id_from_parents(
        &self,
        adv: &dyn spur_pm::BeadsAdvanced,
        issue: &spur_pm::IssueSummary,
        parents: &[String],
    ) -> anyhow::Result<Option<String>> {
        let mut parents = parents.to_vec();
        parents.sort();
        parents.dedup();
        if parents.len() > 1 {
            anyhow::bail!(
                "issue '{}' has multiple structural parents: {}",
                issue.id,
                parents.join(", ")
            );
        }
        let Some(parent_id) = parents.first() else {
            return Ok(None);
        };

        let comments = adv.list_comments(parent_id).await?;
        let audits = crate::plan::projector::collect_sorted_audits_for_issue(parent_id, comments)?;
        Ok(expected_plan_id_from_audits(&audits))
    }
}
