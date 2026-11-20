//! `screen_record.active{}`: which recording this engine is running right now.
//!
//! Every client (the app window, the desktop shell when it is asked to close,
//! an agent) can see a running recording without knowing its capture id. A
//! recording the person did not start must still be visible to them, and the
//! app must never close over a recording without finishing it.

use super::capture_registry;
use cut_core::{CutError, VerbResult};
use serde_json::{json, Value};

/// Wall-clock milliseconds since the Unix epoch.
pub(crate) fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The JSON projection of the active capture. `kind` is `recording` for a
/// project recording and `rehearsal` for a disposable rehearsal take.
pub(super) fn active_projection() -> Value {
    let Some((capture_id, project_dir, control)) = capture_registry::active_capture_snapshot()
    else {
        return json!({ "active": false, "capture_id": null });
    };
    let readiness = control.readiness_status();
    json!({
        "active": true,
        "capture_id": capture_id,
        "kind": if project_dir.is_some() { "recording" } else { "rehearsal" },
        "project_path": project_dir.map(|dir| dir.display().to_string()),
        "ready": readiness.ready,
        "stop_requested": control.stop_requested(),
        "first_screen_frame_unix_ms": readiness.first_screen_frame_unix_ms,
        "since_first_screen_frame_ms": readiness
            .first_screen_frame_unix_ms
            .map(|first| unix_ms_now().saturating_sub(first)),
    })
}

/// The engine is shutting down (Ctrl-C, SIGTERM, or its desktop shell went
/// away) while a project recording runs: stop it and wait until its worker has
/// saved it, so the take is finished instead of cut off. A recording saved this
/// way keeps its next-start notice ("saved when the app closed"). The desktop
/// shell holds a window close until a recording is saved, so this path is for
/// a shell that vanished or a stop that came from outside.
pub(crate) async fn finish_for_shutdown(limit: std::time::Duration) {
    let Some((capture_id, Some(project_dir), _control)) =
        capture_registry::active_capture_snapshot()
    else {
        return;
    };
    tracing::info!(capture_id = %capture_id, "finishing the running recording before shutdown");
    super::stop_capture(&capture_id);
    let started = std::time::Instant::now();
    while capture_registry::is_capture_active(&capture_id) && started.elapsed() < limit {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let capture_dir = project_dir
        .join("cache")
        .join("screen_record")
        .join(&capture_id);
    let saved = record_recovery::recovery_status(&capture_dir)
        .receipt
        .is_some_and(|receipt| receipt.state == record_recovery::RecoveryState::Complete);
    if saved {
        super::pending_recordings::after_stop_saved(&capture_id, true);
    }
}

pub(crate) async fn active_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {}
    let _: Args = crate::dispatch::parse_args(args)?;
    Ok(VerbResult::ok(active_projection()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_waits_for_the_recording_to_be_saved_and_keeps_its_notice() {
        let _lock = capture_registry::capture_test_lock().lock().await;
        let project = tempfile::tempdir().unwrap();
        let id = format!("cap_{}_{}_9", std::process::id(), unix_ms_now());
        let reservation =
            capture_registry::reserve_project_transition_test_capture(project.path(), id.clone());
        let project_dir = capture_registry::active_capture_snapshot()
            .unwrap()
            .1
            .unwrap();
        let capture = project_dir.join("cache").join("screen_record").join(&id);
        std::fs::create_dir_all(&capture).unwrap();
        record_recovery::ManifestOwner::begin(
            &capture,
            record_recovery::CaptureStart::new(&id, 15_000),
        )
        .unwrap();
        std::fs::write(capture.join("source.mp4"), b"saved").unwrap();
        super::super::pending_recordings::note_started(&id, project.path());
        let finishing = tokio::spawn(finish_for_shutdown(std::time::Duration::from_secs(30)));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !finishing.is_finished(),
            "shutdown must wait for the recording"
        );
        // The worker saves the recording and releases its reservation.
        record_recovery::ManifestOwner::open(&capture)
            .unwrap()
            .publish_receipt(record_recovery::RecoveryReceipt {
                state: record_recovery::RecoveryState::Complete,
                recovered_segments: 0,
                lost_tail_ms: Some(0),
                lost_tail_lower_bound_ms: 0,
                lost_tail_upper_bound_ms: Some(0),
                audio_first_packet_offset_ms: None,
                source: Some("source.mp4".into()),
                note: "test".into(),
            })
            .unwrap();
        drop(reservation);
        tokio::time::timeout(std::time::Duration::from_secs(5), finishing)
            .await
            .unwrap()
            .unwrap();
        let listed = super::super::pending_recordings::pending_handler(json!({}))
            .await
            .unwrap()
            .result
            .unwrap();
        let item = listed["recordings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["capture_id"] == json!(id))
            .cloned()
            .expect("the close-saved recording is reported at the next start");
        assert_eq!(item["situation"], json!("saved_on_close"));
        super::super::pending_recordings::remove(&id);
    }

    #[tokio::test]
    async fn a_running_recording_is_visible_without_its_capture_id() {
        let _lock = capture_registry::capture_test_lock().lock().await;
        let idle = active_handler(json!({})).await.unwrap();
        assert_eq!(idle.result.as_ref().unwrap()["active"], json!(false));
        let project = tempfile::tempdir().unwrap();
        let reservation = capture_registry::reserve_project_transition_test_capture(
            project.path(),
            "cap_test_active_1",
        );
        let busy = active_handler(json!({})).await.unwrap();
        let busy = busy.result.unwrap();
        assert_eq!(busy["active"], json!(true));
        assert_eq!(busy["capture_id"], json!("cap_test_active_1"));
        assert_eq!(busy["kind"], json!("recording"));
        assert_eq!(busy["ready"], json!(false));
        assert_eq!(busy["first_screen_frame_unix_ms"], Value::Null);
        drop(reservation);
        let after = active_handler(json!({})).await.unwrap();
        assert_eq!(after.result.unwrap()["active"], json!(false));
    }
}
