//! Capture-derived arguments for the stop → autoedit hand-off.
//!
//! A recorder plan is a timebase contract: its frame rate must match the
//! finalized capture, not the auto-edit engine's generic demo default.

use cut_core::{error_codes, CutError};
use serde_json::{json, Value};

pub(crate) fn for_capture(
    events_path: String,
    recording_project: &Value,
    webcam: Option<&str>,
    studio_events: Option<&str>,
) -> Result<Value, CutError> {
    let fps = recording_project
        .get("settings")
        .and_then(|settings| settings.get("fps"))
        .and_then(Value::as_f64)
        .filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps))
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "capture project.json is missing a valid settings.fps",
                "the finalized capture must declare an FPS between 1 and 240 before auto-edit can preserve its timebase",
            )
            .with_suggested_action("retry the recording so it writes a complete project.json")
        })?;
    let mut args = json!({
        "track": events_path,
        "config": {"out_fps": fps},
    });
    let map = args
        .as_object_mut()
        .expect("capture autoedit arguments are an object");
    if let Some(webcam) = webcam {
        map.insert("webcam".into(), Value::String(webcam.into()));
    }
    if let Some(studio_events) = studio_events {
        map.insert("studio_events".into(), Value::String(studio_events.into()));
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::for_capture;
    use serde_json::json;

    #[test]
    fn forwards_the_recorded_fps_into_autoedit_config() {
        let args = for_capture(
            "capture/events.json".into(),
            &json!({"settings":{"fps":25.0}}),
            Some("capture/webcam.mp4"),
            Some("capture/studio-events.json"),
        )
        .unwrap();
        assert_eq!(args["config"]["out_fps"], 25.0);
        assert_eq!(args["webcam"], "capture/webcam.mp4");
        assert_eq!(args["studio_events"], "capture/studio-events.json");
    }

    #[test]
    fn rejects_an_absent_or_invalid_capture_timebase() {
        for project in [json!({}), json!({"settings":{"fps":0}})] {
            assert!(for_capture("events.json".into(), &project, None, None).is_err());
        }
    }
}
