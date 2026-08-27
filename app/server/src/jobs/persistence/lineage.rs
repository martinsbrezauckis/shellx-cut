//! Retry-lineage reconciliation for durable job recovery.

use super::persist;
use crate::jobs::{JobRecord, JobRetryDescriptor, JobState};
use cut_core::CutError;
use std::collections::HashMap;
use std::path::Path;

/// Validate persisted retry lineage and complete the only interrupted
/// admission window: a durable child without a durable consumed parent.
///
/// Recovery deliberately refuses the whole attach on an inconsistent graph.
/// Quarantining one record would leave another visible record pointing at
/// history that no longer exists, while rewriting it would hide evidence of
/// the inconsistency.
pub(super) fn reconcile_retry_lineage(
    dir: &Path,
    records: &mut [JobRecord],
) -> Result<(), CutError> {
    let record_indices = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.job_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut children_by_parent = HashMap::<String, Vec<usize>>::new();

    for (child_index, record) in records.iter().enumerate() {
        if let Some(retry) = record.retry.as_ref() {
            if retry.retry_of.is_none()
                && (retry.root_job_id != record.job_id || retry.attempt != 1)
            {
                return Err(retry_lineage_error(
                    "inconsistent persisted retry lineage",
                    format!(
                        "retry root '{}' must name itself and use attempt 1",
                        record.job_id
                    ),
                ));
            }
            if let Some(descriptor) = retry.descriptor.as_ref() {
                if !retry_descriptor_matches_kind(record, descriptor) {
                    return Err(retry_lineage_error(
                        "inconsistent persisted retry lineage",
                        format!(
                            "retry descriptor owner does not match canonical screen-record export job kind for '{}'",
                            record.job_id
                        ),
                    ));
                }
            }
        }
        let Some(parent_id) = record
            .retry
            .as_ref()
            .and_then(|retry| retry.retry_of.as_deref())
        else {
            continue;
        };
        children_by_parent
            .entry(parent_id.to_string())
            .or_default()
            .push(child_index);
    }

    let mut repairs = Vec::new();
    for child in records.iter() {
        let Some(child_retry) = child.retry.as_ref() else {
            continue;
        };
        let Some(parent_id) = child_retry.retry_of.as_deref() else {
            continue;
        };
        let parent_index = record_indices.get(parent_id).copied().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' references missing parent '{parent_id}'",
                    child.job_id
                ),
            )
        })?;
        let parent = &records[parent_index];
        let parent_retry = parent.retry.as_ref().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' references parent '{}' without a retry projection",
                    child.job_id, parent.job_id
                ),
            )
        })?;
        let root_index = record_indices
            .get(child_retry.root_job_id.as_str())
            .copied()
            .ok_or_else(|| {
                retry_lineage_error(
                    "inconsistent persisted retry lineage",
                    format!(
                        "retry job '{}' references missing root '{}'",
                        child.job_id, child_retry.root_job_id
                    ),
                )
            })?;
        let root = &records[root_index];
        let root_retry = root.retry.as_ref().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' references root '{}' without a retry projection",
                    child.job_id, root.job_id
                ),
            )
        })?;

        if children_by_parent
            .get(parent_id)
            .is_some_and(|children| children.len() != 1)
        {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "parent '{}' has multiple persisted retry children",
                    parent.job_id
                ),
            ));
        }
        if !matches!(parent.state, JobState::Failed) {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!("retry parent '{}' is not a failed record", parent.job_id),
            ));
        }
        if child.kind != parent.kind {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' kind '{}' does not match parent '{}' kind '{}'",
                    child.job_id, child.kind, parent.job_id, parent.kind
                ),
            ));
        }
        if child_retry.root_job_id != parent_retry.root_job_id {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' root '{}' does not match parent '{}' root '{}'",
                    child.job_id, child_retry.root_job_id, parent.job_id, parent_retry.root_job_id
                ),
            ));
        }
        if root_retry.retry_of.is_some()
            || root_retry.root_job_id != root.job_id
            || root_retry.attempt != 1
        {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' root '{}' is not a canonical retry root",
                    child.job_id, root.job_id
                ),
            ));
        }
        if parent_retry.attempt.checked_add(1) != Some(child_retry.attempt) {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' attempt {} does not follow parent '{}' attempt {}",
                    child.job_id, child_retry.attempt, parent.job_id, parent_retry.attempt
                ),
            ));
        }
        let child_descriptor = child_retry.descriptor.as_ref().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!("retry job '{}' has no retry descriptor", child.job_id),
            )
        })?;
        let parent_descriptor = parent_retry.descriptor.as_ref().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!("retry parent '{}' has no retry descriptor", parent.job_id),
            )
        })?;
        if child_descriptor != parent_descriptor {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "retry job '{}' descriptor does not match parent '{}'",
                    child.job_id, parent.job_id
                ),
            ));
        }
        if let Some(retried_by) = parent_retry.retried_by.as_deref() {
            if retried_by != child.job_id {
                return Err(retry_lineage_error(
                    "inconsistent persisted retry lineage",
                    format!(
                        "parent '{}' points to retry child '{}' instead of '{}'",
                        parent.job_id, retried_by, child.job_id
                    ),
                ));
            }
        }

        // A retry child is authoritative once its complete descriptor and
        // lineage validate. Complete the source transition before exposing
        // recovery, whether the interrupted write left it eligible, linkless,
        // or both.
        if !parent_retry.is_marked_retried_by(&child.job_id) {
            repairs.push((parent_index, child.job_id.clone()));
        }
    }

    // Catch dangling/one-sided back-links as well as child-driven checks
    // above. This is intentionally before any repair write.
    for parent in records.iter() {
        let Some(retried_by) = parent
            .retry
            .as_ref()
            .and_then(|retry| retry.retried_by.as_deref())
        else {
            continue;
        };
        let child_index = record_indices.get(retried_by).copied().ok_or_else(|| {
            retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "parent '{}' points to missing retry child '{retried_by}'",
                    parent.job_id
                ),
            )
        })?;
        let child = &records[child_index];
        if child
            .retry
            .as_ref()
            .and_then(|retry| retry.retry_of.as_deref())
            != Some(parent.job_id.as_str())
        {
            return Err(retry_lineage_error(
                "inconsistent persisted retry lineage",
                format!(
                    "parent '{}' points to '{}' but that record does not retry the parent",
                    parent.job_id, child.job_id
                ),
            ));
        }
    }

    for (parent_index, child_id) in repairs {
        let parent = &mut records[parent_index];
        let retry = parent
            .retry
            .as_mut()
            .expect("validated retry parent has a retry projection");
        retry.mark_retried(&child_id);
        parent.updated_ts = cut_core::OpRecord::now_ts();
        persist(&dir.join(format!("{}.json", parent.job_id)), parent)?;
    }

    Ok(())
}

fn retry_descriptor_matches_kind(record: &JobRecord, descriptor: &JobRetryDescriptor) -> bool {
    matches!(
        (record.kind.as_str(), descriptor),
        (
            "screen_record_export",
            JobRetryDescriptor::ScreenRecordExport(_)
        )
    )
}

fn retry_lineage_error(message: impl Into<String>, cause: impl Into<String>) -> CutError {
    CutError::new(cut_core::error_codes::CONFLICT, message, cause)
        .with_suggested_action("inspect the persisted retry lineage before reopening the project")
}
