use super::*;
use crate::state::AppState;

fn plan_with_two_targets() -> CachePurgePlan {
    CachePurgePlan {
        plan_id: "cache_plan_1".into(),
        project_dir: std::path::PathBuf::from("/private/project.cutproj"),
        project_revision: Some("op_000001".into()),
        created_ms: 1,
        roots: Vec::new(),
        snapshot: Vec::new(),
        targets: vec![
            FileIdentity {
                kind: CacheKind::Proxies,
                name: "a1.mp4".into(),
                bytes: 40,
                modified_ms: 1,
            },
            FileIdentity {
                kind: CacheKind::Thumbnails,
                name: "a1.jpg".into(),
                bytes: 60,
                modified_ms: 1,
            },
        ],
        before: CacheMeasurement {
            files: 2,
            bytes: 100,
        },
    }
}

#[test]
fn cancelled_purge_reconciliation_keeps_partial_progress_exact() {
    let plan = plan_with_two_targets();
    let result = reconciliation_result(
        &plan,
        1,
        40,
        CacheMeasurement {
            files: 1,
            bytes: 60,
        },
        ReconciliationStatus::Cancelled,
        false,
        false,
    );
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["reconciliation"]["planned"]["files"], 2);
    assert_eq!(result["reconciliation"]["removed"]["bytes"], 40);
    assert_eq!(result["reconciliation"]["after"]["bytes"], 60);
    assert_eq!(result["reconciliation"]["balanced"], true);
    assert_eq!(result["reconciliation"]["partial_progress"], true);
    assert_eq!(
        result["reconciliation"]["after_basis"],
        "exclusive_lease_delta"
    );
}

#[tokio::test]
async fn project_switch_cancellation_retains_exact_delta_only_after_lease() {
    let state = AppState::new();
    let job = state.jobs.create("cache_purge");
    let plan = plan_with_two_targets();
    cancel_with_reconciliation(
        &state,
        &job.job_id,
        crate::jobs::JobCancellationReason::ProjectSwitch,
        &plan,
        1,
        40,
        true,
    )
    .await;
    let record = state.jobs.get(&job.job_id).unwrap();
    assert_eq!(record.outcome, Some(crate::jobs::JobOutcome::Cancelled));
    assert_eq!(
        record.outcome_reason,
        Some(crate::jobs::JobOutcomeReason::ProjectSwitchCancelled)
    );
    let reconciliation = &record.result.as_ref().unwrap()["reconciliation"];
    assert_eq!(reconciliation["after"]["bytes"], 60);
    assert_eq!(reconciliation["after_basis"], "exclusive_lease_delta");
    assert_eq!(reconciliation["balanced"], true);

    let before_lease = state.jobs.create("cache_purge");
    cancel_with_reconciliation(
        &state,
        &before_lease.job_id,
        crate::jobs::JobCancellationReason::ProjectSwitch,
        &plan,
        0,
        0,
        false,
    )
    .await;
    let before_lease = state.jobs.get(&before_lease.job_id).unwrap();
    assert_eq!(
        before_lease.outcome,
        Some(crate::jobs::JobOutcome::Cancelled)
    );
    assert_eq!(
        before_lease.outcome_reason,
        Some(crate::jobs::JobOutcomeReason::ProjectSwitchCancelled)
    );
    assert!(before_lease.result.is_none());
}

#[test]
fn failed_ledger_publish_keeps_known_unlink_accounting_fail_closed() {
    let mut plan = plan_with_two_targets();
    plan.targets.truncate(1);
    plan.before = CacheMeasurement {
        files: 1,
        bytes: 40,
    };
    let after = derived_after(&plan, 1, 40).unwrap();
    let result = reconciliation_result(
        &plan,
        1,
        40,
        after,
        ReconciliationStatus::Failed,
        false,
        true,
    );
    assert_eq!(result["status"], "failed");
    assert_eq!(result["reconciliation"]["after"]["files"], 0);
    assert_eq!(result["reconciliation"]["balanced"], true);
    assert_eq!(result["reconciliation"]["ledger_recovery_required"], true);
}
