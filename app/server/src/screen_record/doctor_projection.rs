//! Field-stable `screen_record.doctor` projection and capability admission.
//!
//! Native enumeration owns the opaque identity. This server layer copies it
//! exactly and deliberately does not derive replacement identities from display
//! copy, ordinal, primary status, or geometry.

use super::{
    align_ffmpeg_env, camera_public, microphone, recording_controls, recording_scenes,
    start_readiness,
};
use crate::dispatch::parse_args;
use cut_core::{CutError, VerbResult};
use serde::Serialize;
use serde_json::{json, Value};

/// One capability card (kept field-stable for the `screen_record.doctor` result
/// the UI and agent already consume). `unknown` is never ready/green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordStartAdmission {
    Strict,
    LinuxPortalPromptDeferred,
    /// A status read did not check screen delivery; Start runs that check.
    CheckedAtStart,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordCard {
    pub name: String,
    pub status: String,
    pub detail: String,
    /// Server-only action provenance; clients receive explicit admission.
    #[serde(skip_serializing)]
    pub(crate) start_admission: RecordStartAdmission,
}

/// One display the user can pick as the capture target. Its opaque `id` is
/// passed unchanged to `screen_record.start` when present.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MonitorInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// One application window the user can pick for the in-app window picker.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub app: String,
}

/// Every capability card plus strict readiness and action-specific admission.
#[derive(Debug, Clone, Serialize)]
pub struct RecordDoctor {
    pub cards: Vec<RecordCard>,
    pub ready: bool,
    /// Plain screen/sound admission; opt-in keystrokes are checked separately at Start.
    pub start_allowed: bool,
    pub monitors: Vec<MonitorInfo>,
    pub windows: Vec<WindowInfo>,
    pub window_capture_supported: bool,
    pub quality: record_capture::CaptureQualityCapability,
}

pub(super) fn required_capture_card(cards: &[RecordCard], name: &str) -> bool {
    matches!(name, "ffmpeg" | "screen_capture" | "input_hook")
        || (matches!(name, "gstreamer" | "wayland_input" | "video_encoder")
            && cards.iter().any(|card| card.name == name))
}

/// `ready` rollup from a card list. Linux adds platform-specific requirements.
pub(super) fn ready_rollup(cards: &[RecordCard]) -> bool {
    [
        "ffmpeg",
        "screen_capture",
        "input_hook",
        "gstreamer",
        "wayland_input",
        "video_encoder",
    ]
    .into_iter()
    .filter(|name| required_capture_card(cards, name))
    .all(|name| {
        cards
            .iter()
            .any(|card| card.name == name && card.status == "ok")
    })
}

pub(super) fn apply_capture_access_failure(cards: &mut [RecordCard]) {
    let Some(card) = cards
        .iter_mut()
        .find(|card| card.name == "screen_capture" && card.status == "ok")
    else {
        return;
    };
    card.status = "degraded".into();
    card.detail = "Screen capture permission is unavailable — allow ShellX Cut in System Settings > Privacy & Security > Screen & System Audio Recording (Screen Recording on older macOS), then quit and reopen the app".into();
}

/// In-process capability cards. A native backend is `ok` only after it delivers
/// a discarded frame to Cut; `unknown` is not ready. On Windows and macOS this
/// runs a short real capture: use it only for Check, Test, or a start.
pub(super) fn doctor() -> RecordDoctor {
    align_ffmpeg_env();
    let cards: Vec<RecordCard> = record_capture::doctor()
        .into_iter()
        .map(record_card)
        .collect();
    project(cards, true)
}

/// The same projection for a status display. It never starts a capture and
/// never shows a system prompt: the screen card is the last check or "not
/// checked yet", and macOS source lists are read only once the OS already
/// allows screen capture.
pub(super) fn doctor_status() -> (RecordDoctor, record_capture::ScreenCheck) {
    align_ffmpeg_env();
    let status = record_capture::doctor_status();
    let cards = status
        .cards
        .into_iter()
        .map(record_card)
        .map(|mut card| {
            if card.name == "screen_capture"
                && card.status == "unknown"
                && status.screen_check == record_capture::ScreenCheck::NotChecked
            {
                card.start_admission = RecordStartAdmission::CheckedAtStart;
            }
            card
        })
        .collect();
    let may_list = status.screen_check != record_capture::ScreenCheck::PermissionNeeded;
    (project(cards, may_list), status.screen_check)
}

fn project(mut cards: Vec<RecordCard>, may_list: bool) -> RecordDoctor {
    // Preserve ScreenCaptureKit/TCC enumeration failures. The old Vec-only API
    // collapsed permission denial to an empty picker while leaving ready=true.
    let monitor_probe = if may_list {
        record_capture::list_monitors_checked()
    } else {
        Ok(Vec::new())
    };
    if monitor_probe.is_err() {
        apply_capture_access_failure(&mut cards);
    }
    let ready = ready_rollup(&cards);
    let start_allowed = start_readiness::start_allowed(&cards);
    let monitors = monitors(monitor_probe.unwrap_or_default());
    let windows = may_list
        .then(record_capture::list_windows)
        .unwrap_or_default()
        .into_iter()
        .map(|window| WindowInfo {
            id: window.id,
            title: window.title,
            app: window.app,
        })
        .collect();
    let quality = record_capture::capture_quality_capability();
    RecordDoctor {
        cards,
        ready,
        start_allowed,
        monitors,
        windows,
        window_capture_supported: record_capture::window_capture_supported(),
        quality,
    }
}

pub(super) fn record_card(card: record_capture::Card) -> RecordCard {
    let start_admission =
        if record_capture::is_linux_portal_prompt_deferred(&card.status, &card.detail) {
            RecordStartAdmission::LinuxPortalPromptDeferred
        } else {
            RecordStartAdmission::Strict
        };
    RecordCard {
        name: card.id,
        status: card.status,
        detail: card.detail,
        start_admission,
    }
}

pub(super) async fn screen_record_doctor(args: Value) -> Result<VerbResult, CutError> {
    let args: super::video_encoder_readiness::DoctorArgs = parse_args(args)?;
    let request = args.encoder.request()?;
    let mode = args.mode.as_deref().unwrap_or("check");
    if !matches!(mode, "check" | "status" | "microphone") {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            format!("unknown Doctor mode `{mode}`"),
            "mode must be check, status, or microphone",
        )
        .with_suggested_action("omit mode for a full check, or pass status or microphone"));
    }
    let mic_warm = if args.warm_mic {
        // Keep a potentially unbounded native-driver call off the async runtime.
        let task = tokio::task::spawn_blocking(|| {
            // Held for the native work itself, which can outlive the 4 s wait below.
            let _test = super::audio_test_gate::AudioTestGuard::enter();
            microphone::warm_projection()
        });
        Some(
            match tokio::time::timeout(std::time::Duration::from_secs(4), task).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => json!({
                    "live": false,
                    "supported": true,
                    "error": format!("microphone warm-up worker failed: {error}"),
                }),
                Err(_) => json!({
                    "live": false,
                    "supported": true,
                    "timed_out": true,
                    "error": "microphone warm-up exceeded 4 seconds",
                }),
            },
        )
    } else {
        None
    };
    if mode == "microphone" {
        #[cfg(target_os = "linux")]
        let readiness = microphone::readiness::MicrophoneReadiness::current_linux().await;
        #[cfg(not(target_os = "linux"))]
        let readiness = microphone::readiness::MicrophoneReadiness::current();
        return Ok(VerbResult::ok(microphone::readiness::projection(
            readiness, mic_warm,
        )));
    }
    let (mut doctor, screen_check) = if mode == "status" {
        doctor_status()
    } else {
        (doctor(), checked_screen())
    };
    let encoder = super::video_encoder_readiness::apply(&mut doctor, request, mode != "status");
    let microphone = microphone::doctor_projection();
    #[cfg(target_os = "linux")]
    let readiness = microphone::readiness::MicrophoneReadiness::current_linux().await;
    #[cfg(not(target_os = "linux"))]
    let readiness = microphone::readiness::MicrophoneReadiness::current();
    Ok(VerbResult::ok(super::video_encoder_readiness::projection(
        json!({
            "mode": mode,
            "microphone_access": readiness.access,
            "microphone_blocked": microphone::readiness::blocked_projection(readiness),
            "screen_check": screen_check,
            "cards": doctor.cards,
            "ready": doctor.ready,
            "start_allowed": doctor.start_allowed,
            "monitors": doctor.monitors,
            "windows": doctor.windows,
            "window_capture_supported": doctor.window_capture_supported,
            "quality": doctor.quality,
            "mic_warm": mic_warm,
            "microphones": microphone.microphones,
            "microphone_selection": microphone.microphone_selection,
            "camera": camera_public::capability(),
            "scenes": recording_scenes::capability(),
            "pause": recording_controls::capability(),
            "region_selection": {
                "supported": false,
                "detail": "Area capture remains unavailable pending compiled/native qualification of the foreground picker path. The private macOS source bridge is not exposed; Windows and Linux have no crop backend.",
            },
        }),
        encoder,
    )))
}

/// What a full check reports about screen delivery: Linux Doctor never runs
/// one (its portal asks at Start), every other backend just did.
fn checked_screen() -> record_capture::ScreenCheck {
    if cfg!(target_os = "linux") {
        record_capture::ScreenCheck::NotApplicable
    } else {
        record_capture::ScreenCheck::Checked
    }
}

pub(super) fn monitors(
    source: impl IntoIterator<Item = record_capture::MonitorInfo>,
) -> Vec<MonitorInfo> {
    source
        .into_iter()
        .map(|monitor| MonitorInfo {
            id: monitor.id,
            index: monitor.index,
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            primary: monitor.primary,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_the_opaque_native_id_without_rederiving_it() {
        let id = "shellx-monitor-v1:windows:0123456789abcdef";
        let result = monitors([record_capture::MonitorInfo {
            id: Some(id.into()),
            index: 2,
            name: "Renamed display".into(),
            width: 1_920,
            height: 1_080,
            primary: false,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_deref(), Some(id));
        assert_eq!(result[0].index, 2);
        assert_eq!(result[0].name, "Renamed display");
    }

    #[test]
    fn projection_keeps_an_identity_unavailable_row_usable_by_the_legacy_picker() {
        let result = monitors([record_capture::MonitorInfo {
            id: None,
            index: 3,
            name: "Display with no exact identity".into(),
            width: 1_280,
            height: 720,
            primary: false,
        }]);
        assert_eq!(result[0].id, None);
        assert_eq!(result[0].index, 3);
    }
}
