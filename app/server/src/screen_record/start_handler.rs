//! Public `screen_record.start` admission and capture reservation.

use super::*;
use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::VerbResult;

/// Start a live, bounded or open-ended screen recording. A selected camera is
/// an explicit Use-camera action; passive Doctor discovery never reaches the
/// native camera owner.
pub(crate) async fn screen_record_start(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        duration_ms: Option<u64>,
        fps: Option<f64>,
        quality: Option<record_core::CaptureQualityRequest>,
        #[serde(default)]
        audio: bool,
        #[serde(default)]
        system_audio: bool,
        studio: Option<Value>,
        #[serde(default)]
        keys: bool,
        monitor: Option<u32>,
        monitor_id: Option<String>,
        window: Option<String>,
        camera_id: Option<String>,
        scenes: Option<record_capture::RecordingSceneConfig>,
        pause: Option<recording_controls::PauseStart>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let duration_ms = a.duration_ms;
    let fps = a.fps.unwrap_or(30.0);
    validate_capture_settings(duration_ms, fps)?;
    let quality_requested = a.quality.is_some();
    let capture_quality = record_capture::admit_capture_quality(a.quality).map_err(record_err)?;
    let cadence = cadence::from_server_fps(fps)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let recorder_doctor = doctor();
    start_readiness::ensure_start_ready(&recorder_doctor.cards)?;
    let monitor_target = monitor_start_admission::admit(
        a.monitor,
        a.monitor_id.as_deref(),
        a.window.is_some(),
        &recorder_doctor.monitors,
    )?;
    let pause_enabled = recording_controls::admit_public_start(
        a.pause,
        fps,
        quality_requested,
        a.keys,
        &monitor_target,
        a.window.is_some(),
        a.camera_id.is_some(),
        a.scenes.is_some(),
    )?;
    let microphone_source = if a.audio {
        microphone::source_for_start()?
    } else {
        record_capture::MicrophoneSource::SystemDefault
    };
    if let Some(camera_id) = a.camera_id.as_deref() {
        camera_public::admit_selected(camera_id)?;
    }
    let recording_scene_camera_admitted = a.camera_id.is_some();
    let recording_scene_snapshot = (!pause_enabled)
        .then(|| recording_scenes::admit_start_config(a.scenes, recording_scene_camera_admitted))
        .transpose()?;
    let recording_scene_start = recording_scene_snapshot
        .as_ref()
        .map(recording_scenes::start_projection);

    let capture_id = new_capture_id();
    windows_path::ensure_pre_marker_path(&dir, &capture_id)?;
    let recovery_scan = recovery::scan_recovery_for_project(&dir)?;
    let out_dir = create_capture_dir(&dir, &capture_id)?;
    recovery::begin(&out_dir, &capture_id)?;
    let project_path = capture_file(&dir, &capture_id, "project.json")?;
    let marker_body = json!({
        "pid": std::process::id(),
        "duration_ms": duration_ms,
        "open_ended": duration_ms.is_none(),
        "fps": fps,
        "cadence": cadence,
        "quality": capture_quality,
        "audio": a.audio,
        "system_audio": a.system_audio,
        "studio": a.studio,
        "keys": a.keys,
        "camera_id": a.camera_id,
        "scenes": recording_scene_start.clone(),
        "pause": pause_enabled.then(|| json!({ "mode": "enabled" })),
    });
    publish_marker(
        &dir,
        &capture_id,
        &serde_json::to_vec_pretty(&marker_body).unwrap_or_default(),
    )?;

    let record_log = out_dir.join("record.log");
    let start_result = if pause_enabled {
        #[cfg(target_os = "macos")]
        {
            macos_pause_capture::start_public_selected(
                capture_id.clone(),
                duration_ms,
                fps,
                monitor_target.legacy_index,
                monitor_target.exact_id,
                dir.clone(),
                out_dir.clone(),
                project_path.clone(),
                record_log,
                microphone_source,
                a.audio,
                a.system_audio,
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            unreachable!("pause admission is unavailable outside macOS")
        }
    } else {
        start_capture_with_recording_scenes(
            capture_id.clone(),
            duration_ms,
            fps,
            capture_quality,
            a.audio,
            microphone_source,
            a.system_audio,
            a.keys,
            monitor_target.legacy_index,
            monitor_target.exact_id,
            a.window,
            None,
            a.camera_id,
            recording_scene_snapshot,
            recording_scene_camera_admitted,
            dir.clone(),
            out_dir.clone(),
            project_path.clone(),
            record_log,
        )
    };
    if let Err(error) = start_result {
        let _ = std::fs::remove_dir_all(&out_dir);
        return Err(error);
    }

    Ok(VerbResult::ok(json!({
        "capture_id": capture_id,
        "out_dir": out_dir,
        "status": "recording",
        "duration_ms": duration_ms,
        "open_ended": duration_ms.is_none(),
        "cadence": cadence,
        "scenes": recording_scene_start,
        "pause": { "enabled": pause_enabled },
        "studio_events": crate::screen_record_studio::studio_events_path(&out_dir),
        "recovery_scan": {
            "recovered": recovery_scan.recovered,
            "deferred": recovery_scan.deferred,
            "failed_closed": recovery_scan.failed_closed,
        },
        "note": if duration_ms.is_none() {
            "OPEN-ENDED capture: runs until screen_record.stop. The platform may show its one-time screen or camera permission prompt on the desktop."
        } else {
            "capture runs up to duration_ms (or until screen_record.stop). The platform may show its one-time screen or camera permission prompt on the desktop."
        },
    })))
}

/// `screen_record.status{capture_id}` — report only the live first-frame
/// admission fact. This reads process-local reservation state and never opens a
/// native source, touches a capture path, or changes capture lifecycle.
pub(crate) async fn readiness_status_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct StatusArgs {
        capture_id: String,
    }

    let args: StatusArgs = parse_args(args)?;
    let control = capture_registry::active_capture_control(&args.capture_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "screen recording is not active in this cutd process",
            "the capture may have finalized or this cutd process may have restarted",
        )
        .with_suggested_action("start a new recording and retain its returned capture_id")
    })?;
    let status = control.readiness_status();
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "ready": status.ready,
        "terminal": status.terminal,
        "state": status.state.as_str(),
    })))
}

#[cfg(test)]
mod readiness_status_tests {
    use super::*;

    #[tokio::test]
    async fn status_requires_a_delivered_frame_and_never_admits_a_terminal_capture() {
        let _capture_lock = super::capture_test_lock().lock().await;
        let capture_id = format!("cap_readiness_{}", std::process::id());
        let control = CaptureSessionControl::new(None, false, false, false);
        let readiness = control.readiness();
        let reservation = reserve_capture(capture_id.clone(), control.clone()).unwrap();

        let pending = readiness_status_handler(json!({"capture_id": capture_id}))
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(pending["state"], "awaiting_first_screen_frame");
        assert_eq!(pending["ready"], false);
        assert_eq!(pending["terminal"], false);

        readiness.mark_first_screen_frame_delivered();
        let ready = readiness_status_handler(json!({"capture_id": capture_id}))
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(ready["state"], "ready");
        assert_eq!(ready["ready"], true);
        assert_eq!(ready["terminal"], false);

        control.terminalize().unwrap();
        let terminal = readiness_status_handler(json!({"capture_id": capture_id}))
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(terminal["state"], "terminal_after_first_screen_frame");
        assert_eq!(terminal["ready"], false);
        assert_eq!(terminal["terminal"], true);

        drop(reservation);
        let missing = readiness_status_handler(json!({"capture_id": capture_id}))
            .await
            .unwrap_err();
        assert_eq!(missing.code, error_codes::NOT_FOUND);
    }
}
