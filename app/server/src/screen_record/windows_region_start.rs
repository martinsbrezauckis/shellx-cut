//! Private Windows Region ticket consumption.
//!
//! The ticket remains child-only, correlated, and topology-bound. After atomic
//! burn and revalidation it enters the ordinary recorder ownership path with a
//! private native crop; the Windows backend performs that crop on Direct3D
//! textures and maps input against the same subsurface.

use super::region_selection::RegionSelectionTicket;
use super::windows_region_bridge;
use super::{
    capture_file, create_capture_dir, doctor, monitor_start_admission, new_capture_id,
    publish_marker, record_err, recovery, start_capture, start_readiness,
    validate_capture_settings, windows_path,
};
use crate::dispatch::snapshot;
use crate::state::AppState;
use cut_core::{CutError, VerbResult};
use serde_json::{json, Value};

/// Non-serializable ticket minted only after the authenticated foreground
/// Windows bridge. No coordinates or raw target values can reach this owner.
pub(super) struct PrivateWindowsRegionStart {
    pub(super) duration_ms: Option<u64>,
    pub(super) fps: f64,
    pub(super) quality: Option<record_core::CaptureQualityRequest>,
    pub(super) audio: bool,
    pub(super) system_audio: bool,
    pub(super) keys: bool,
    pub(super) studio: Option<Value>,
    pub(super) ticket: RegionSelectionTicket,
}

/// Burn and revalidate before any normal start work, then use exactly the same
/// readiness, reservation, recovery, marker, and worker path as Display.
pub(super) async fn start_from_foreground_ticket(
    state: &AppState,
    request: PrivateWindowsRegionStart,
) -> Result<VerbResult, CutError> {
    // Keep the project identity stable from private ticket consumption through
    // ordinary capture reservation, matching the public start path.
    let _project_transition = state.project_transition.lock().await;
    let (monitor_id, crop) = windows_region_bridge::consume_ticket(&request.ticket)?;
    validate_capture_settings(request.duration_ms, request.fps)?;
    let quality = record_capture::admit_capture_quality(request.quality).map_err(record_err)?;
    let cadence = super::cadence::from_server_fps(request.fps)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let recorder_doctor = doctor();
    start_readiness::ensure_start_ready(&recorder_doctor.cards)?;
    let target =
        monitor_start_admission::admit(None, Some(&monitor_id), false, &recorder_doctor.monitors)?;
    let microphone_source = if request.audio {
        super::microphone::source_for_start()?
    } else {
        record_capture::MicrophoneSource::SystemDefault
    };

    let capture_id = new_capture_id();
    windows_path::ensure_pre_marker_path(&dir, &capture_id)?;
    let recovery_scan = recovery::scan_recovery_for_project(&dir)?;
    let out_dir = create_capture_dir(&dir, &capture_id)?;
    recovery::begin(&out_dir, &capture_id)?;
    let project_path = capture_file(&dir, &capture_id, "project.json")?;
    let marker_body = json!({
        "pid": std::process::id(),
        "duration_ms": request.duration_ms,
        "open_ended": request.duration_ms.is_none(),
        "fps": request.fps,
        "cadence": cadence,
        "quality": quality,
        "audio": request.audio,
        "system_audio": request.system_audio,
        "studio": request.studio,
        "keys": request.keys,
        "source": "region",
    });
    publish_marker(
        &dir,
        &capture_id,
        &serde_json::to_vec_pretty(&marker_body).unwrap_or_default(),
    )?;

    let record_log = out_dir.join("record.log");
    if let Err(error) = start_capture(
        capture_id.clone(),
        request.duration_ms,
        request.fps,
        quality,
        request.audio,
        microphone_source,
        request.system_audio,
        request.keys,
        target.legacy_index,
        target.exact_id,
        None,
        Some(crop),
        None,
        dir.clone(),
        out_dir.clone(),
        project_path,
        record_log,
    ) {
        let _ = std::fs::remove_dir_all(&out_dir);
        return Err(error);
    }

    Ok(VerbResult::ok(json!({
        "capture_id": capture_id,
        "out_dir": out_dir,
        "status": "recording",
        "duration_ms": request.duration_ms,
        "open_ended": request.duration_ms.is_none(),
        "cadence": cadence,
        "studio_events": crate::screen_record_studio::studio_events_path(&out_dir),
        "recovery_scan": {
            "recovered": recovery_scan.recovered,
            "deferred": recovery_scan.deferred,
            "failed_closed": recovery_scan.failed_closed,
        },
        "note": "private Windows Region capture runs until its duration bound or screen_record.stop",
    })))
}
