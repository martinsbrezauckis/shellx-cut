use super::*;

#[tokio::test]
async fn duplicate_stop_keeps_options_and_one_pending_job() {
    let state = AppState::new();
    let key = (
        std::sync::Arc::as_ptr(&state.project_transition) as usize,
        PathBuf::from("C:\\recording-test"),
        "capture_test".to_string(),
    );
    let options = StopOptions {
        autoedit: false,
        mux_raw: true,
        raw_path: Some("C:\\recording-test\\first.mp4".into()),
    };
    lock_pending().entries.insert(
        key.clone(),
        StopJobEntry {
            job_id: "job_17".into(),
            options: options.clone(),
            finished: false,
        },
    );
    let duplicate = existing_stop(&state, &key, &options, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(duplicate.result.unwrap()["job_id"], "job_17");
    let changed = StopOptions {
        raw_path: Some("C:\\recording-test\\second.mp4".into()),
        ..options
    };
    assert_eq!(
        existing_stop(&state, &key, &changed, false)
            .await
            .unwrap()
            .unwrap_err()
            .code,
        error_codes::CONFLICT
    );
    mark_finished(&key);
    assert!(existing_stop(&state, &key, &changed, false).await.is_none());
    let mut jobs = lock_pending();
    jobs.entries.remove(&key);
    jobs.completed.retain(|completed| completed != &key);
}

#[test]
fn only_nonpause_marker_uses_background_save() {
    let dir = tempfile::tempdir().unwrap();
    let id = "cap_test";
    let capture_dir = dir.path().join("cache/screen_record").join(id);
    std::fs::create_dir_all(&capture_dir).unwrap();
    let marker = capture_dir.join(".capture.json");
    std::fs::write(&marker, br#"{"pause":null}"#).unwrap();
    assert!(is_ordinary_marker(dir.path(), id));
    std::fs::write(&marker, br#"{"pause":{"mode":"enabled"}}"#).unwrap();
    assert!(!is_ordinary_marker(dir.path(), id));
}

#[tokio::test]
async fn panic_is_reported_failed_before_project_lock_is_released() {
    let state = AppState::new();
    let job_id = state.jobs.create("screen_record_stop").job_id;
    let guard = state.project_transition.clone().lock_owned().await;
    let outer_state = state.clone();
    let outer_job_id = job_id.clone();
    let watcher = tokio::spawn(async move {
        let _guard = guard;
        let worker = tokio::spawn(async {
            panic!("stand-in save failure");
            #[allow(unreachable_code)]
            Ok(VerbResult::ok(json!({})))
        });
        let result = settle_stop_job(&outer_state, &outer_job_id, worker).await;
        assert!(outer_state
            .project_transition
            .clone()
            .try_lock_owned()
            .is_err());
        assert_eq!(
            outer_state.jobs.get(&outer_job_id).unwrap().state,
            crate::jobs::JobState::Failed
        );
        result
    });
    let error = watcher.await.unwrap().unwrap_err();
    assert_eq!(error.code, error_codes::JOB_FAILED);
    assert!(state.project_transition.clone().try_lock_owned().is_ok());
}
