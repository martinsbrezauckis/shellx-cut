//! Public admission truth for the bounded macOS pause-safe owner.

use super::monitor_start_admission::Target;
use cut_core::{error_codes, CutError};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PauseMode {
    Enabled,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PauseStart {
    pub(super) mode: PauseMode,
}

pub(super) fn capability() -> Value {
    if cfg!(target_os = "macos") {
        json!({
            "supported": true,
            "layouts": ["screen"],
            "streams": ["screen_video", "microphone_audio", "system_audio"],
            "requires": ["exact_display", "integer_fps"],
            "incompatible": [
                { "feature": "scenes", "reason": "public Recording Scenes require their own receipt publication and cannot share the prepublished pause owner" },
                { "feature": "camera", "reason": "the pause owner does not seal a camera stream" },
                { "feature": "keys", "reason": "the pause owner does not seal passive input events" },
                { "feature": "window", "reason": "the pause owner admits one exact display, not an application window" },
                { "feature": "quality", "reason": "the pause owner does not own a verified quality transform" }
            ],
            "recovery": "versioned durable pause journal; Pause and Resume acknowledge only sealed transitions, while Stop retains the existing terminal projection and recovery receipt",
            "detail": "On macOS, enable Pause & resume for one exact display at an integer frame rate. Screen plus optional microphone and system audio are sealed together before an acknowledgement."
        })
    } else {
        json!({
            "supported": false,
            "incompatible": [{ "feature": "platform", "reason": "public Pause & resume is currently available only on macOS" }],
            "detail": "Pause and resume are unavailable on this platform. Stop remains available to finalize an ordinary recording."
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn admit_public_start(
    requested: Option<PauseStart>,
    fps: f64,
    quality_requested: bool,
    keys: bool,
    target: &Target,
    has_window: bool,
    has_camera: bool,
    has_scenes: bool,
) -> Result<bool, CutError> {
    let Some(requested) = requested else {
        return Ok(false);
    };
    let PauseMode::Enabled = requested.mode;
    if !cfg!(target_os = "macos") {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "Pause and resume are available only on macOS",
            "start an ordinary recording on this platform",
        ));
    }
    if target.exact_id.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Pause & resume requires an exact display selected from Recorder",
            "refresh Recorder and choose a listed display with its current identity",
        ));
    }
    if !fps.is_finite() || fps.fract() != 0.0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Pause & resume requires an integer frame rate",
            "choose a whole-number frame rate such as 30 or 60 fps",
        ));
    }
    if has_scenes {
        return incompatible(
            "Recording Scenes",
            "turn off Pause & resume or record without Scenes",
        );
    }
    if has_camera {
        return incompatible(
            "camera recording",
            "turn off Pause & resume or record screen-only",
        );
    }
    if keys {
        return incompatible(
            "keystroke capture",
            "turn off Pause & resume or turn off Show keystrokes",
        );
    }
    if has_window {
        return incompatible(
            "window recording",
            "choose Display while Pause & resume is enabled",
        );
    }
    if quality_requested {
        return incompatible(
            "quality selection",
            "clear Quality while Pause & resume is enabled",
        );
    }
    Ok(true)
}

fn incompatible(feature: &str, action: &str) -> Result<bool, CutError> {
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        format!("Pause & resume cannot be combined with {feature}"),
        action,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(exact: bool) -> Target {
        Target {
            legacy_index: Some(1),
            exact_id: exact.then(|| "current-monitor".into()),
        }
    }

    #[test]
    fn pause_capability_names_the_exact_public_limits() {
        let capability = capability();
        if cfg!(target_os = "macos") {
            assert_eq!(capability["supported"], true);
            assert_eq!(capability["requires"][0], "exact_display");
            assert_eq!(capability["incompatible"][0]["feature"], "scenes");
        } else {
            assert_eq!(capability["supported"], false);
        }
    }

    #[test]
    fn non_macos_pause_start_refuses_before_any_reservation() {
        let result = admit_public_start(
            Some(PauseStart {
                mode: PauseMode::Enabled,
            }),
            30.0,
            false,
            false,
            &target(true),
            false,
            false,
            false,
        );
        if cfg!(target_os = "macos") {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().code, error_codes::NOT_FOUND);
        }
    }
}
