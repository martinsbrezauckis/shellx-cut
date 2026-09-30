use std::path::Path;

use super::super::dispatch;
use super::test_actor;
use crate::jobs::{
    JobInputFingerprint, JobRetry, ScreenRecordExportRetryDescriptor, ScreenRecordExportRetryFormat,
};
use crate::state::AppState;
use serde_json::json;
use sha2::{Digest, Sha256};

fn fingerprint(role: &str, path: &Path) -> JobInputFingerprint {
    let bytes = std::fs::read(path).expect("retry fixture input");
    JobInputFingerprint {
        role: role.into(),
        path: path
            .file_name()
            .expect("project-relative fixture input")
            .to_string_lossy()
            .into_owned(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

async fn failed_recorder_export(state: &AppState, project_dir: &Path) -> String {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": "jobs_retry", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);

    let source = project_dir.join("source.mp4");
    let plan = project_dir.join("edit.json");
    std::fs::write(
        &source,
        b"not a media fixture; retry only needs its exact fingerprint",
    )
    .unwrap();
    std::fs::write(
        &plan,
        serde_json::to_vec(&record_core::EditPlan::empty(160, 90, 1000, 25.0)).unwrap(),
    )
    .unwrap();
    let revision = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .log
            .read_all()
            .unwrap()
            .last()
            .map(|operation| operation.op_id.clone())
            .unwrap_or_else(|| "op_000000".into())
    };
    let job = state.jobs.create_with_retry(
        "screen_record_export",
        Some(JobRetry::screen_record_export(
            ScreenRecordExportRetryDescriptor {
                project_revision: revision,
                source: "source.mp4".into(),
                plan: "edit.json".into(),
                format: ScreenRecordExportRetryFormat::Mp4,
                inputs: vec![
                    fingerprint("edit_plan", &plan),
                    fingerprint("recording_source", &source),
                ],
                system_audio_offset_ms: 0,
            },
        )),
    );
    state.jobs.fail(
        &job.job_id,
        cut_core::CutError::new("job_failed", "fixture export failure", "fixture"),
    );
    job.job_id
}

#[tokio::test]
async fn jobs_retry_admits_one_linked_recorder_child_and_hides_the_recipe() {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let parent_id = failed_recorder_export(&state, &temp.path().join("jobs_retry.cutproj")).await;

    let retried = dispatch(
        &state,
        "jobs.retry",
        json!({"job_id": parent_id}),
        test_actor(),
    )
    .await;
    assert!(retried.ok, "retry failed: {:?}", retried.error);
    let result = retried.result.unwrap();
    let child_id = result["job_id"].as_str().unwrap().to_string();
    assert_eq!(result["retry_of"], parent_id);
    assert_eq!(result["root_job_id"], parent_id);
    assert_eq!(result["attempt"], 2);
    assert_eq!(result["status"], "queued");

    let parent = state.jobs.get(&parent_id).unwrap();
    assert_eq!(
        parent.retry.unwrap().retried_by.as_deref(),
        Some(child_id.as_str())
    );
    let second = dispatch(
        &state,
        "jobs.retry",
        json!({"job_id": parent_id}),
        test_actor(),
    )
    .await;
    assert!(!second.ok, "a source must admit at most one retry child");
    assert_eq!(second.error.unwrap().code, cut_core::error_codes::CONFLICT);

    let status = dispatch(
        &state,
        "jobs.status",
        json!({"job_id": child_id}),
        test_actor(),
    )
    .await;
    assert!(status.ok);
    assert!(status.result.unwrap()["retry"].get("descriptor").is_none());
    let _ = state.jobs.abort(&child_id).await;
}

#[tokio::test]
async fn jobs_retry_refuses_changed_recorder_inputs_before_output_admission() {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("jobs_retry.cutproj");
    let parent_id = failed_recorder_export(&state, &project_dir).await;
    std::fs::write(
        project_dir.join("source.mp4"),
        b"changed after the failed export",
    )
    .unwrap();

    let retried = dispatch(
        &state,
        "jobs.retry",
        json!({"job_id": parent_id}),
        test_actor(),
    )
    .await;
    assert!(!retried.ok, "changed retry input must not be admitted");
    assert_eq!(retried.error.unwrap().code, cut_core::error_codes::CONFLICT);
    assert!(state.jobs.get(&parent_id).unwrap().retry.unwrap().eligible);
    assert_eq!(state.jobs.list().len(), 1, "retry must not create a child");
    assert!(!project_dir.join("exports/recording.mp4").exists());
}
