//! Row 184: opening a project with an interrupted recording answers at once;
//! the recording is prepared by a visible `recording_recovery` job.

use std::time::Duration;

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use serde_json::json;

#[tokio::test]
async fn project_open_starts_a_visible_preparation_job_instead_of_recovering_inline() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rec.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "rec", "dir": path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    // A capture without a receipt, as a forced quit leaves it.
    let capture = path.join("cache/screen_record/cap_1_2_0");
    std::fs::create_dir_all(&capture).unwrap();
    record_recovery::ManifestOwner::begin(
        &capture,
        record_recovery::CaptureStart::new("cap_1_2_0", 15_000),
    )
    .unwrap();

    let opened = dispatch(&state, "project.open", json!({"path": path}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let scan = &opened.result.as_ref().unwrap()["recovery_scan"];
    assert_eq!(scan["state"], "running", "{scan}");
    let job_id = scan["job_id"].as_str().unwrap().to_string();

    let mut record = None;
    for _ in 0..200 {
        let status = dispatch(
            &state,
            "jobs.status",
            json!({"job_id": job_id}),
            test_actor(),
        )
        .await;
        let result = status.result.unwrap_or_default();
        if result["state"] == "done" || result["state"] == "failed" {
            record = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let record = record.expect("the preparation job finishes");
    assert_eq!(record["kind"], "recording_recovery");
    // This process still owns the capture, so it is deferred, never touched.
    assert_eq!(record["result"]["deferred"][0], "cap_1_2_0", "{record}");

    // A project with nothing to prepare starts no job.
    let quiet = root.path().join("quiet.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "quiet", "dir": quiet}),
        test_actor(),
    )
    .await;
    assert!(created.ok);
    let opened = dispatch(&state, "project.open", json!({"path": quiet}), test_actor()).await;
    assert_eq!(opened.result.unwrap()["recovery_scan"]["state"], "none");
}
