//! Public Recording Scenes admission, capability, and live-switch dispatch.
//!
//! This module deliberately owns no camera device, renderer, or journal path.
//! It validates the bounded public layout vocabulary, delegates durable state to
//! record-capture, and exposes only a current deterministic projection.

use super::capture_registry::active_capture_control;
use super::{existing_capture_dir, recovery};
use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use record_capture::RecordingSceneTimerAction;
use record_core::scene_projection::{
    SceneFixtureDescriptor, SceneFixtureRenderer, SceneTimerFixture,
};
use record_core::{AcceptedStartSnapshot, PresetRevision, SceneId, SceneState, TimerPhase};
use serde_json::{json, Value};

pub(super) fn admit_start_config(
    config: Option<record_capture::RecordingSceneConfig>,
    camera_selected: bool,
) -> Result<AcceptedStartSnapshot, CutError> {
    let config = config.unwrap_or_else(record_capture::RecordingSceneConfig::default_screen);
    let initial_requires_camera = config.initial_requires_camera().map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Recording Scenes catalog is invalid",
            error.to_string(),
        )
    })?;
    if initial_requires_camera && !camera_selected {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Presenter PiP scenes require an explicitly selected camera",
            "the requested scene catalog contains presenter_pip; start with a current screen_record.doctor camera id",
        )
        .with_suggested_action(
            "select a ready Camera in Recorder, or use Screen-only scenes",
        ));
    }
    config.accepted_snapshot().map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Recording Scenes catalog is invalid",
            error.to_string(),
        )
        .with_suggested_action(
            "use 1..32 uniquely named Screen or Presenter PiP presets with non-zero revisions",
        )
    })
}

/// Passive, path-free capability.  It never opens a camera or capture source;
/// Presenter PiP readiness is conditional on a later explicit camera choice.
pub(super) fn capability() -> Value {
    json!({
        "supported": cfg!(any(unix, windows)),
        "catalog_revision": 1,
        "layouts": ["screen", "presenter_pip"],
        "timers": ["off", "elapsed", "countdown"],
        "timer_actions": ["pause", "resume", "reset", "restart", "end"],
        "max_presets": record_core::MAX_SCENES,
        "camera_required_layouts": ["presenter_pip"],
        "recovery": {
            "journal": "versioned_jsonl",
            "torn_tail": "exclusive_owner_repairs_only_an_unambiguously_incomplete_final_record",
            "malformed_or_replaced_leaf": "fails_closed",
        },
        "detail": if cfg!(any(unix, windows)) {
            "Screen and Presenter PiP layouts are durable CaptureClock-timed scene events. A Presenter catalog may start Screen-only without a camera; initial or live Presenter PiP requires an explicitly selected admitted camera."
        } else {
            "Recording Scenes are unavailable because this platform has no durable journal namespace barrier."
        },
    })
}

pub(super) fn start_projection(snapshot: &AcceptedStartSnapshot) -> Value {
    let state = SceneState::initial(snapshot)
        .expect("screen_record.start only calls this with an accepted scene snapshot");
    let scene = SceneFixtureRenderer::render(snapshot, &state, 0)
        .expect("accepted recording scene snapshot must render at logical time zero");
    json!({
        "enabled": true,
        "saved": true,
        "scene_id": snapshot.initial_scene_id().as_str(),
        "preset_revision": snapshot.revision().initial_preset.get(),
        "catalog_revision": snapshot.revision().catalog.get(),
        "logical_media_time_ms": 0,
        "timer_started": false,
        "timer": timer_config_value(snapshot.timer_config().expect("accepted timer")),
        "scene": scene,
    })
}

pub(super) async fn screen_record_scene_activate(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
        scene_id: String,
        preset_revision: u64,
    }

    let args: Args = parse_args(args)?;
    recovery::validate_capture_id(&args.capture_id)?;
    let scene_id = SceneId::parse(args.scene_id).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Recording Scene id is invalid",
            error.to_string(),
        )
    })?;
    let preset_revision = PresetRevision::new(args.preset_revision).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Recording Scene preset revision must be non-zero",
            error.to_string(),
        )
    })?;
    let acknowledged_scene_id = scene_id.as_str().to_owned();
    let (_project, _edl, project_dir, _at) = snapshot(state).await?;
    if existing_capture_dir(&project_dir, &args.capture_id)?.is_none() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "Recording Scene capture is not in the open project",
            "the capture_id does not name a local capture directory under this project",
        ));
    }
    let control = active_capture_control(&args.capture_id).ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "Recording Scene capture is not active in this cutd process",
            "scene switching is available only while this exact capture remains live; terminal projection is published at stop",
        )
    })?;
    let (logical_media_time_ms, scene) = control
        .activate_recording_scene(scene_id, preset_revision)
        .map_err(|error| {
            CutError::new(
                error_codes::CONFLICT,
                "Recording Scene could not switch",
                error.to_string(),
            )
        })?;
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "saved": true,
        "scene_id": acknowledged_scene_id,
        "preset_revision": preset_revision.get(),
        "logical_media_time_ms": logical_media_time_ms,
        "scene": scene,
    })))
}

/// Public live timer control. The durable owner maps its bounded vocabulary to
/// the generic reducer at the current CaptureClock timestamp.
pub(super) async fn screen_record_scene_timer(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    enum Action {
        Pause,
        Resume,
        Reset,
        Restart,
        End,
    }

    impl Action {
        fn name(&self) -> &'static str {
            match self {
                Self::Pause => "pause",
                Self::Resume => "resume",
                Self::Reset => "reset",
                Self::Restart => "restart",
                Self::End => "end",
            }
        }

        fn engine(&self) -> RecordingSceneTimerAction {
            match self {
                Self::Pause => RecordingSceneTimerAction::Pause,
                Self::Resume => RecordingSceneTimerAction::Resume,
                Self::Reset => RecordingSceneTimerAction::Reset,
                Self::Restart => RecordingSceneTimerAction::Restart,
                Self::End => RecordingSceneTimerAction::End,
            }
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
        action: Action,
    }

    let args: Args = parse_args(args)?;
    recovery::validate_capture_id(&args.capture_id)?;
    let (_project, _edl, project_dir, _at) = snapshot(state).await?;
    if existing_capture_dir(&project_dir, &args.capture_id)?.is_none() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "Recording Scene capture is not in the open project",
            "the capture_id does not name a local capture directory under this project",
        ));
    }
    let control = active_capture_control(&args.capture_id).ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "Recording Scene capture is not active in this cutd process",
            "timer control is available only while this exact capture remains live",
        )
    })?;
    let (logical_media_time_ms, scene) = control
        .recording_scene_timer_action(args.action.engine())
        .map_err(|error| {
            CutError::new(
                error_codes::CONFLICT,
                "Recording Scene timer could not switch",
                error.to_string(),
            )
        })?;
    let timer_state = timer_state(&scene).ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "Recording Scene timer could not acknowledge a live state",
            "the requested action left no running, paused, or ended timer state",
        )
    })?;
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "action": args.action.name(),
        "state": timer_state,
        "logical_media_time_ms": logical_media_time_ms,
        "scene": scene,
    })))
}

fn timer_config_value(config: record_core::TimerConfig) -> Value {
    match config {
        record_core::TimerConfig::Off => json!({"kind": "off"}),
        record_core::TimerConfig::Elapsed => json!({"kind": "elapsed"}),
        record_core::TimerConfig::Countdown { duration_ms } => {
            json!({"kind": "countdown", "duration_ms": duration_ms.get()})
        }
    }
}

fn timer_state(scene: &SceneFixtureDescriptor) -> Option<&'static str> {
    match scene.timer {
        SceneTimerFixture::Elapsed { phase, .. } | SceneTimerFixture::Countdown { phase, .. } => {
            match phase {
                TimerPhase::Running | TimerPhase::TimeUp => Some("running"),
                TimerPhase::Paused => Some("paused"),
                TimerPhase::Ended => Some("ended"),
                TimerPhase::Off | TimerPhase::Ready => None,
            }
        }
        SceneTimerFixture::Off => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_is_passive_and_declares_only_renderable_layouts() {
        let capability = capability();
        assert_eq!(capability["layouts"], json!(["screen", "presenter_pip"]));
        assert_eq!(capability["catalog_revision"], 1);
        assert_eq!(capability["timers"], json!(["off", "elapsed", "countdown"]));
        assert_eq!(
            capability["camera_required_layouts"],
            json!(["presenter_pip"])
        );
        assert_eq!(capability["recovery"]["journal"], "versioned_jsonl");
    }

    #[test]
    fn presenter_catalog_fails_closed_without_explicit_camera_selection() {
        let config: record_capture::RecordingSceneConfig = serde_json::from_value(json!({
            "catalog_revision": 1,
            "initial_scene_id": "presenter",
            "presets": [{
                "id": "presenter",
                "name": "Presenter",
                "preset_revision": 1,
                "layout": {
                    "kind": "presenter_pip",
                    "corner": "top_right",
                    "size_percent": 25,
                    "shape": "rounded_rect"
                }
            }],
            "timer": {"kind": "elapsed"}
        }))
        .unwrap();
        let error = admit_start_config(Some(config), false).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("camera"));
    }

    #[test]
    fn screen_initial_catalog_may_include_presenter_without_a_camera() {
        let config: record_capture::RecordingSceneConfig = serde_json::from_value(json!({
            "catalog_revision": 1,
            "initial_scene_id": "screen",
            "presets": [{
                "id": "screen", "name": "Screen", "preset_revision": 1,
                "layout": {"kind": "screen"}
            }, {
                "id": "presenter", "name": "Presenter", "preset_revision": 2,
                "layout": {"kind":"presenter_pip","corner":"top_right","size_percent":25,"shape":"circle"}
            }],
            "timer": {"kind": "elapsed"}
        }))
        .unwrap();
        assert_eq!(
            admit_start_config(Some(config), false)
                .unwrap()
                .initial_scene_id()
                .as_str(),
            "screen"
        );
    }
}
