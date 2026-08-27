use cut_core::error_codes;
use serde_json::{json, Value};
use std::path::Path;

const FILE: &str = "microphone-outcome.json";

pub(crate) fn persist(
    capture_dir: &Path,
    outcome: record_capture::MicrophoneCaptureOutcome,
) -> record_core::Result<()> {
    let (status, mic_track_saved) = match outcome {
        record_capture::MicrophoneCaptureOutcome::NotRequested => ("not_requested", false),
        record_capture::MicrophoneCaptureOutcome::Saved => ("saved", true),
        record_capture::MicrophoneCaptureOutcome::MicrophoneLostSavedPrefix => {
            ("microphone_lost", true)
        }
        record_capture::MicrophoneCaptureOutcome::MicrophoneLostNoTrack => {
            ("microphone_lost", false)
        }
    };
    let body = serde_json::to_vec(&json!({"schema": "shellx-cut/microphone-outcome/1", "status": status, "mic_track_saved": mic_track_saved}))
        .map_err(|error| record_core::RecordError::new(error_codes::IO, "serialize microphone outcome", error.to_string()))?;
    record_recovery::replace_synced(&capture_dir.join(FILE), &body).map_err(|error| {
        record_core::RecordError::new(
            error_codes::IO,
            "publish microphone outcome",
            error.to_string(),
        )
    })
}

pub(crate) fn projection(capture_dir: &Path) -> Value {
    let fallback = json!({"status": "unavailable", "mic_track_saved": false});
    let Ok(bytes) = std::fs::read(capture_dir.join(FILE)) else {
        return fallback;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return fallback;
    };
    match (
        value.get("status").and_then(Value::as_str),
        value.get("mic_track_saved").and_then(Value::as_bool),
    ) {
        (Some("not_requested" | "saved" | "microphone_lost"), Some(mic_track_saved)) => {
            json!({"status": value["status"], "mic_track_saved": mic_track_saved})
        }
        _ => fallback,
    }
}
