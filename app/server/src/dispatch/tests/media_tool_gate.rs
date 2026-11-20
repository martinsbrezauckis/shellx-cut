//! Verbs refuse up front, with no state left behind, when ffmpeg is missing.

use super::test_actor;
use crate::dispatch::dispatch;
use crate::media_tool_gate::{test_presence, MEDIA_TOOL_VERBS};
use crate::state::AppState;
use serde_json::{json, Value};

#[test]
fn every_gated_name_is_a_real_verb() {
    let state = AppState::new();
    for verb in MEDIA_TOOL_VERBS {
        assert!(
            state.registry.get(verb).is_some(),
            "{verb} is not in schema/verbs.json"
        );
    }
}

/// Assets, op count and job count of the open project, to prove a refusal
/// changed nothing.
async fn footprint(state: &AppState) -> (Value, usize, usize) {
    let project = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(project.ok, "{:?}", project.error);
    let project = project.result.unwrap();
    let assets = project
        .get("project")
        .and_then(|p| p.get("assets"))
        .or_else(|| project.get("assets"))
        .cloned()
        .unwrap_or(Value::Null);
    let ops = dispatch(state, "project.ops", json!({}), test_actor()).await;
    let op_count = ops
        .result
        .as_ref()
        .and_then(|r| r.get("ops"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let jobs = dispatch(state, "jobs.list", json!({}), test_actor()).await;
    let job_count = jobs
        .result
        .as_ref()
        .and_then(|r| r.get("jobs"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    (assets, op_count, job_count)
}

#[tokio::test]
async fn media_verbs_refuse_with_the_reason_and_leave_nothing_behind() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "no-ffmpeg", "dir": root.path().join("no-ffmpeg.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let clip = root.path().join("clip.mp4");
    std::fs::write(&clip, b"not really video").unwrap();
    let before = footprint(&state).await;

    let _missing = test_presence::missing();
    for (verb, args) in [
        ("media.import", json!({"path": clip})),
        ("render.frame", json!({"at_ms": 0})),
        ("render.final", json!({})),
        ("render.final", json!({"dry_run": true})),
        ("export.frame", json!({"at_ms": 0})),
    ] {
        let r = dispatch(&state, verb, args, test_actor()).await;
        assert!(!r.ok, "{verb} must refuse without ffmpeg");
        let error = r.error.expect("an error");
        assert_eq!(
            error.code,
            cut_core::error_codes::TOOL_MISSING,
            "{verb}: {error:?}"
        );
        assert_eq!(error.message, "ffmpeg is not installed", "{verb}");
        let action = error.suggested_action.expect("a way forward");
        assert!(action.contains("system."), "{verb}: {action}");
    }
    assert_eq!(
        footprint(&state).await,
        before,
        "a refusal must change nothing"
    );
}

#[tokio::test]
async fn verbs_that_do_not_need_ffmpeg_still_work_without_it() {
    let _missing = test_presence::missing();
    let state = AppState::new();
    let r = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
}

/// A step whose filter no installed build has is refused before it changes
/// the project, with the feature, the library and the way to get it.
#[tokio::test]
async fn steps_needing_a_missing_filter_refuse_up_front() {
    use cut_media::toolpath::FfmpegFeature;
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "filters", "dir": root.path().join("filters.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let before = footprint(&state).await;

    let _missing =
        test_presence::features_missing(&[FfmpegFeature::Vidstab, FfmpegFeature::Zscale]);
    let r = dispatch(
        &state,
        "edit.stabilize",
        json!({"clip": "c1"}),
        test_actor(),
    )
    .await;
    let error = r.error.expect("refused");
    assert_eq!(error.code, cut_core::error_codes::TOOL_MISSING);
    assert!(
        error
            .message
            .starts_with("Stabilize needs FFmpeg with vid.stab"),
        "{}",
        error.message
    );
    assert!(error.suggested_action.unwrap().contains("system.doctor"));

    let r = dispatch(
        &state,
        "project.color",
        json!({"output": "rec2020"}),
        test_actor(),
    )
    .await;
    let error = r.error.expect("refused");
    assert!(error.message.contains("zimg"), "{}", error.message);
    assert_eq!(
        footprint(&state).await,
        before,
        "a refusal must change nothing"
    );

    // Turning Stabilize off needs no filter: the gate lets it through to the
    // handler (which then reports the unknown clip).
    let r = dispatch(
        &state,
        "edit.stabilize",
        json!({"clip": "c1", "enabled": false}),
        test_actor(),
    )
    .await;
    assert_ne!(
        r.error.map(|e| e.code),
        Some(cut_core::error_codes::TOOL_MISSING.to_string())
    );
    // A rec709 project color needs nothing either.
    let r = dispatch(
        &state,
        "project.color",
        json!({"output": "rec709"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
}
