//! Tests for Start-owned fallback project creation, one-use rollback,
//! forged creation-id rejection, and no-project refused Start contracts.

use super::*;
use crate::screen_record::microphone::readiness::{MicrophoneReadiness, TestReadiness};
use record_capture::MicrophoneAccess;

const ISOLATED_ROLLBACK_CHILD_ENV: &str = "SHELLX_CUT_REFUSED_PROJECT_ROLLBACK_CHILD";

fn run_isolated_rollback_parent(test_name: &str) -> (tempfile::TempDir, PathBuf) {
    let temp_home = tempfile::tempdir().unwrap();
    let projects_dir = temp_home.path().join("Projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    // The child runs Start's capture doctor; with native capture off it never asks the OS for
    // screens (on macOS a ScreenCaptureKit list outside a desktop session can wait forever).
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(ISOLATED_ROLLBACK_CHILD_ENV, "1")
        .env("SHELLX_CUT_NATIVE_CAPTURE_OFF", "1")
        .env("SHELLX_CUT_HOME", temp_home.path())
        .env("SHELLX_CUT_PROJECTS_DIR", &projects_dir)
        .status()
        .unwrap();
    assert!(status.success(), "isolated child test failed");
    (temp_home, projects_dir)
}

fn denied_mic() -> MicrophoneReadiness {
    MicrophoneReadiness {
        compiled: true,
        access: MicrophoneAccess::Denied,
        windows_switch: None,
        no_device: false,
    }
}

#[tokio::test]
async fn forged_creation_id_is_rejected_and_existing_open_project_is_never_discarded() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    record_capture::forbid_native_capture_access();
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("existing-empty.cutproj");
    let state = AppState::new();

    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "existing-empty", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    assert_eq!(
        created.op_ids.as_deref(),
        Some(&["op_000001".to_string()][..])
    );

    let forged_start = dispatch(
        &state,
        "screen_record.start",
        json!({"discard_created_project": "op_000001"}),
        test_actor(),
    )
    .await;
    assert!(!forged_start.ok);
    assert_eq!(forged_start.error.unwrap().code, error_codes::INVALID_ARGS);

    let forged_delete = dispatch(
        &state,
        "project.delete",
        json!({"path": project_dir, "discard_empty_created_by": "op_000001"}),
        test_actor(),
    )
    .await;
    assert!(!forged_delete.ok);
    assert_eq!(forged_delete.error.unwrap().code, error_codes::INVALID_ARGS);

    let _mic = TestReadiness::install(denied_mic());
    let refused = dispatch(
        &state,
        "screen_record.start",
        json!({"audio": true, "system_audio": false, "fps": 30}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    assert!(project_dir.join("ops.jsonl").is_file());
    assert!(project_dir.join("project.json").is_file());
    let open_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(
        open_state.ok,
        "existing open project must stay open: {:?}",
        open_state.error
    );
}

#[test]
fn schema_exposes_screen_record_start_optional_project_state_and_no_discard_arg() {
    let schema =
        serde_json::from_str::<Value>(include_str!("../../../../../schema/verbs.json")).unwrap();
    let verbs = schema["verbs"].as_array().unwrap();
    let start = verbs
        .iter()
        .find(|v| v["name"] == "screen_record.start")
        .expect("screen_record.start in schema");
    assert_eq!(
        start["behavior"]["project_state"],
        json!("optional"),
        "screen_record.start must allow no open project and create one inside Start"
    );
    assert!(
        start["input_schema"]["properties"]
            .get("discard_created_project")
            .is_none(),
        "no caller creation-id discard option may exist on screen_record.start"
    );
    let delete = verbs
        .iter()
        .find(|v| v["name"] == "project.delete")
        .expect("project.delete in schema");
    assert!(
        delete["input_schema"]["properties"]
            .get("discard_empty_created_by")
            .is_none(),
        "no caller discard option may exist on project.delete"
    );
}

#[tokio::test]
async fn refused_start_without_open_project_leaves_no_empty_project() {
    if std::env::var_os(ISOLATED_ROLLBACK_CHILD_ENV).is_none() {
        let (_home, projects_dir) = run_isolated_rollback_parent(
            "dispatch::tests::screen_record_refused_project_rollback::refused_start_without_open_project_leaves_no_empty_project",
        );
        assert_eq!(std::fs::read_dir(&projects_dir).unwrap().count(), 0);
        return;
    }

    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    record_capture::forbid_native_capture_access();
    let _mic = TestReadiness::install(denied_mic());
    let state = AppState::new();
    let projects_dir = PathBuf::from(std::env::var_os("SHELLX_CUT_PROJECTS_DIR").unwrap());

    let refused = dispatch(
        &state,
        "screen_record.start",
        json!({"audio": true, "system_audio": false, "fps": 30}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    let error = refused.error.expect("refused Start must return error");
    assert_ne!(
        error.code,
        error_codes::NO_PROJECT,
        "Start without expected_project_identity must not fail with no_project: {error:?}"
    );
    let expected_mic_cause = denied_mic()
        .problem()
        .expect("denied_mic has problem")
        .reason;
    assert_eq!(error.cause, expected_mic_cause);
    assert_eq!(
        refused
            .result
            .as_ref()
            .and_then(|r| r.get("created_project_kept")),
        Some(&json!(false)),
        "truthful outcome must report created_project_kept:false: {:?}",
        refused.result
    );
    assert_eq!(std::fs::read_dir(&projects_dir).unwrap().count(), 0);
    assert!(state.project.read().await.is_none());
    let listed = dispatch(&state, "project.list", json!({}), test_actor()).await;
    assert!(listed.ok, "{:?}", listed.error);
    assert_eq!(
        listed.result.unwrap()["projects"].as_array().unwrap().len(),
        0
    );
}

#[tokio::test]
async fn late_refused_start_removes_created_project_while_added_file_or_symlink_swap_is_kept() {
    if std::env::var_os(ISOLATED_ROLLBACK_CHILD_ENV).is_none() {
        let _guard = run_isolated_rollback_parent(
            "dispatch::tests::screen_record_refused_project_rollback::late_refused_start_removes_created_project_while_added_file_or_symlink_swap_is_kept",
        );
        return;
    }

    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let projects_dir = PathBuf::from(std::env::var_os("SHELLX_CUT_PROJECTS_DIR").unwrap());
    let state = AppState::new();

    // Case 1: Late native refusal after fallback project creation + capture dir/marker creation removes everything cleanly.
    let refused = crate::screen_record::screen_record_start_for_rollback_test(
        &state,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(|_dir, _out_dir| {
                Err(CutError::new(
                    error_codes::CONFLICT,
                    "ScreenCast portal was cancelled",
                    "native capture refused after countdown",
                ))
            }),
        },
    )
    .await
    .unwrap();
    assert!(!refused.ok);
    assert_eq!(
        refused.result.as_ref().unwrap()["created_project_discarded"],
        true
    );
    assert_eq!(
        refused.result.as_ref().unwrap()["created_project_kept"],
        false
    );
    assert_eq!(
        refused.result.as_ref().unwrap()["created_project_outcome"],
        "discarded"
    );
    assert_eq!(std::fs::read_dir(&projects_dir).unwrap().count(), 0);
    assert!(state.project.read().await.is_none());

    // Case 2: If an unexpected file appears inside the newly created folder before failure, Start leaves it and reports kept:true.
    let kept_extra = crate::screen_record::screen_record_start_for_rollback_test(
        &state,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(|proj_dir, _out_dir| {
                std::fs::write(proj_dir.join("notes.txt"), "keep me").unwrap();
                Err(CutError::new(
                    error_codes::CONFLICT,
                    "capture refused after adding file",
                    "extra entry exists",
                ))
            }),
        },
    )
    .await
    .unwrap();
    assert!(!kept_extra.ok);
    assert_eq!(
        kept_extra.result.as_ref().unwrap()["created_project_discarded"],
        false
    );
    assert_eq!(
        kept_extra.result.as_ref().unwrap()["created_project_kept"],
        true
    );
    assert_eq!(
        kept_extra.result.as_ref().unwrap()["created_project_outcome"],
        "kept"
    );
    assert!(state.project.read().await.is_some());

    // Case 3: When Start succeeds without an open project, it keeps the created fallback project open and reports created_project:true.
    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(closed.ok, "{:?}", closed.error);
    let admitted = crate::screen_record::screen_record_start_for_rollback_test(
        &state,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(|_dir, _out_dir| Ok(())),
        },
    )
    .await
    .unwrap();
    assert!(admitted.ok);
    assert_eq!(admitted.result.as_ref().unwrap()["created_project"], true);
    assert!(admitted.result.as_ref().unwrap()["project_identity"].is_object());
    assert!(state.project.read().await.is_some());

    // Case 4: Swapping the created project folder for a symlink during Start leaves victim untouched and does not open the symlink.
    #[cfg(unix)]
    {
        let closed2 = dispatch(&state, "project.close", json!({}), test_actor()).await;
        assert!(closed2.ok, "{:?}", closed2.error);
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.cutproj");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("keep.txt"), "safe").unwrap();
        let moved_holder = outside.path().join("moved");
        let victim_clone = victim.clone();
        let swapped = crate::screen_record::screen_record_start_for_rollback_test(
            &state,
            json!({"audio": false, "system_audio": false, "fps": 30}),
            test_actor(),
            crate::screen_record::TestFallbackStartBehavior {
                capture_result: Box::new(move |proj_dir, _out_dir| {
                    std::fs::rename(proj_dir, &moved_holder).unwrap();
                    std::os::unix::fs::symlink(&victim_clone, proj_dir).unwrap();
                    Err(CutError::new(
                        error_codes::CONFLICT,
                        "late failure after symlink swap",
                        "swapped",
                    ))
                }),
            },
        )
        .await
        .unwrap();
        assert!(!swapped.ok);
        assert_eq!(
            swapped.result.as_ref().unwrap()["created_project_discarded"],
            false
        );
        // CUT-122LR-03: the swapped folder cannot be confirmed, so never say "kept".
        assert_eq!(
            swapped.result.as_ref().unwrap()["created_project_kept"],
            false
        );
        assert_eq!(
            swapped.result.as_ref().unwrap()["created_project_outcome"],
            "incomplete"
        );
        assert!(victim.join("keep.txt").is_file());
        assert!(state.project.read().await.is_none());
    }
}

#[tokio::test]
async fn empty_inputs_concurrent_clients_and_restart_preserve_rollback_guarantees() {
    if std::env::var_os(ISOLATED_ROLLBACK_CHILD_ENV).is_none() {
        let _guard = run_isolated_rollback_parent(
            "dispatch::tests::screen_record_refused_project_rollback::empty_inputs_concurrent_clients_and_restart_preserve_rollback_guarantees",
        );
        return;
    }

    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    record_capture::forbid_native_capture_access();
    let projects_dir = PathBuf::from(std::env::var_os("SHELLX_CUT_PROJECTS_DIR").unwrap());
    let state = AppState::new();

    // Empty `{}` and malformed `expected_project_identity: {}` never leak empty projects.
    {
        let _mic = TestReadiness::install(denied_mic());
        let empty = dispatch(&state, "screen_record.start", json!({}), test_actor()).await;
        assert!(!empty.ok);
        let empty_identity = dispatch(
            &state,
            "screen_record.start",
            json!({"expected_project_identity": {}}),
            test_actor(),
        )
        .await;
        assert!(!empty_identity.ok);
        assert_eq!(
            empty_identity.error.unwrap().code,
            error_codes::INVALID_ARGS
        );
        assert_eq!(std::fs::read_dir(&projects_dir).unwrap().count(), 0);
    }

    // Two concurrent refused fallback Starts serialize cleanly via project_transition and leave 0 projects.
    let (r1, r2) = tokio::join!(
        crate::screen_record::screen_record_start_for_rollback_test(
            &state,
            json!({"audio": false, "system_audio": false, "fps": 30}),
            test_actor(),
            crate::screen_record::TestFallbackStartBehavior {
                capture_result: Box::new(|_, _| Err(CutError::new(
                    error_codes::CONFLICT,
                    "c1",
                    "c1"
                ))),
            },
        ),
        crate::screen_record::screen_record_start_for_rollback_test(
            &state,
            json!({"audio": false, "system_audio": false, "fps": 30}),
            test_actor(),
            crate::screen_record::TestFallbackStartBehavior {
                capture_result: Box::new(|_, _| Err(CutError::new(
                    error_codes::CONFLICT,
                    "c2",
                    "c2"
                ))),
            },
        ),
    );
    assert!(!r1.unwrap().ok);
    assert!(!r2.unwrap().ok);
    assert_eq!(std::fs::read_dir(&projects_dir).unwrap().count(), 0);

    // Simulated restart sees 0 projects after clean rollback, and sees only a kept altered folder.
    let restarted_after_discard = AppState::new();
    let listed_none = dispatch(
        &restarted_after_discard,
        "project.list",
        json!({}),
        test_actor(),
    )
    .await;
    assert_eq!(
        listed_none.result.unwrap()["projects"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let kept = crate::screen_record::screen_record_start_for_rollback_test(
        &restarted_after_discard,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(|proj_dir, _| {
                std::fs::write(proj_dir.join("user-note.txt"), "keep").unwrap();
                Err(CutError::new(error_codes::CONFLICT, "kept", "added file"))
            }),
        },
    )
    .await
    .unwrap();
    assert_eq!(kept.result.as_ref().unwrap()["created_project_kept"], true);

    let restarted_after_keep = AppState::new();
    let listed_kept = dispatch(
        &restarted_after_keep,
        "project.list",
        json!({}),
        test_actor(),
    )
    .await;
    let projects = listed_kept.result.unwrap()["projects"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(projects.len(), 1);
    let kept_path = projects[0]["path"].as_str().unwrap().to_string();
    let reopened = dispatch(
        &restarted_after_keep,
        "project.open",
        json!({"path": kept_path}),
        test_actor(),
    )
    .await;
    assert!(reopened.ok, "{:?}", reopened.error);
}

#[cfg(unix)]
#[tokio::test]
async fn failure_after_native_capture_started_carries_capture_id_and_says_take_is_running() {
    if std::env::var_os(ISOLATED_ROLLBACK_CHILD_ENV).is_none() {
        let _guard = run_isolated_rollback_parent(
            "dispatch::tests::screen_record_refused_project_rollback::failure_after_native_capture_started_carries_capture_id_and_says_take_is_running",
        );
        return;
    }
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let state = AppState::new();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.cutproj");
    std::fs::create_dir_all(&victim).unwrap();
    let moved = outside.path().join("moved");
    let victim_clone = victim.clone();
    // The capture "starts" (Ok), then the project folder is swapped so that
    // activation (the later step) fails.
    let reply = crate::screen_record::screen_record_start_for_rollback_test(
        &state,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(move |proj_dir, _out_dir| {
                std::fs::rename(proj_dir, &moved).unwrap();
                std::os::unix::fs::symlink(&victim_clone, proj_dir).unwrap();
                Ok(())
            }),
        },
    )
    .await
    .unwrap();
    assert!(!reply.ok);
    let result = reply
        .result
        .as_ref()
        .expect("reply must carry the running take");
    assert!(result["capture_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert_eq!(result["take_running"], true);
    assert_eq!(result["status"], "recording");
    assert_eq!(result["created_project_kept"], false);
    assert_ne!(result["created_project_outcome"], "not_created");
    let text = serde_json::to_string(&reply.error).unwrap();
    assert!(!text.contains(outside.path().to_str().unwrap()), "{text}");
    assert!(state.project.read().await.is_none());
}

#[tokio::test]
async fn no_project_start_failure_text_is_path_free() {
    if std::env::var_os(ISOLATED_ROLLBACK_CHILD_ENV).is_none() {
        let _guard = run_isolated_rollback_parent(
            "dispatch::tests::screen_record_refused_project_rollback::no_project_start_failure_text_is_path_free",
        );
        return;
    }
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let state = AppState::new();
    let reply = crate::screen_record::screen_record_start_for_rollback_test(
        &state,
        json!({"audio": false, "system_audio": false, "fps": 30}),
        test_actor(),
        crate::screen_record::TestFallbackStartBehavior {
            capture_result: Box::new(|dir, out_dir| {
                Err(CutError::new(
                    error_codes::IO,
                    format!("cannot write {}", out_dir.display()),
                    format!("denied at {}", dir.display()),
                )
                .with_suggested_action(format!("remove {}", dir.display())))
            }),
        },
    )
    .await
    .unwrap();
    assert!(!reply.ok);
    let error = reply.error.unwrap();
    assert_eq!(error.code, error_codes::IO);
    let projects = std::env::var("SHELLX_CUT_PROJECTS_DIR").unwrap();
    let text = serde_json::to_string(&error).unwrap();
    assert!(!text.contains(&projects), "{text}");
    assert!(!text.contains('/'), "{text}");
    assert!(error.suggested_action.is_some());
}
