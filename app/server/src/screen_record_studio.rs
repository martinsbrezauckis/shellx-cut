//! Recording Studio metadata helpers.
//!
//! The live Studio UI writes composition changes beside the raw capture. Those
//! events are metadata only: the recorder keeps raw streams, and the polish pass
//! later replays this file into the `EditPlan.webcam.timeline`.

use std::path::{Path, PathBuf};

use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const STUDIO_EVENTS_FILENAME: &str =
    crate::screen_record_studio_journal::STUDIO_EVENTS_FILENAME;
pub(crate) const LEGACY_STUDIO_EVENTS_FILENAME: &str = "studio-events.json";
const STUDIO_EVENTS_VERSION: u32 = 1;
const MAX_STUDIO_EVENTS_JSON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STUDIO_EVENTS: usize = 50_000;
const MAX_STUDIO_EVENT_T_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StudioEvent {
    /// A server-assigned monotonically increasing acceptance sequence. Legacy
    /// JSON logs omit it; new append-only journal records always carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_ts: Option<u64>,
    pub t_ms: u64,
    pub source: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StudioEventLog {
    pub version: u32,
    pub events: Vec<StudioEvent>,
}

impl Default for StudioEventLog {
    fn default() -> Self {
        Self {
            version: STUDIO_EVENTS_VERSION,
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StudioEventArgs {
    capture_id: String,
    event: StudioEvent,
}

pub(crate) async fn screen_record_studio_event(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let a: StudioEventArgs = parse_args(args)?;
    crate::screen_record::recovery::validate_capture_id(&a.capture_id)?;
    validate_studio_event(&a.event)?;

    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let capture_dir = crate::screen_record::screen_record_cache_dir(&dir)?.join(&a.capture_id);
    if !capture_dir.is_dir() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!(
                "no such capture '{}' ({})",
                a.capture_id,
                capture_dir.display()
            ),
            "the capture_id does not name a capture dir under <project>/cache/screen_record/",
        )
        .with_suggested_action("pass a capture_id returned by screen_record.start"));
    }

    let events_path = studio_events_journal_path(&capture_dir);
    // Marker hotkeys and the Studio background picker are separate async UI
    // paths. They meet at the recorder-owned server writer, which assigns the
    // durable order before append+sync rather than racing a whole-log rewrite.
    let log = state
        .studio_journal
        .lock()
        .await
        .append(&events_path, a.event)?;
    let last_event = log.events.last().cloned().unwrap_or_else(|| StudioEvent {
        logical_ts: None,
        t_ms: 0,
        source: "recording".into(),
        kind: "marker".into(),
        visible: None,
        x: None,
        y: None,
        size: None,
        shape: None,
        radius: None,
        label: None,
        background: None,
    });
    Ok(VerbResult::ok(json!({
        "studio_events": events_path,
        "count": log.events.len(),
        "last_event": last_event,
    })))
}

pub(crate) fn studio_events_path(capture_dir: &Path) -> PathBuf {
    let journal = studio_events_journal_path(capture_dir);
    if journal.exists() || !capture_dir.join(LEGACY_STUDIO_EVENTS_FILENAME).exists() {
        journal
    } else {
        capture_dir.join(LEGACY_STUDIO_EVENTS_FILENAME)
    }
}

fn studio_events_journal_path(capture_dir: &Path) -> PathBuf {
    capture_dir.join(STUDIO_EVENTS_FILENAME)
}

pub(crate) fn read_studio_events(path: &Path) -> Result<StudioEventLog, CutError> {
    if path.file_name().and_then(|name| name.to_str()) == Some(STUDIO_EVENTS_FILENAME) {
        return crate::screen_record_studio_journal::read_studio_journal(path);
    }
    if !path.exists() {
        return Ok(StudioEventLog::default());
    }
    let meta = path.metadata().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not stat {}: {e}", path.display()),
            "reading Studio event metadata failed",
        )
    })?;
    if meta.len() > MAX_STUDIO_EVENTS_JSON_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "studio-events.json is too large: {} bytes (limit: {} bytes)",
                meta.len(),
                MAX_STUDIO_EVENTS_JSON_BYTES
            ),
            format!("oversized Studio event metadata at {}", path.display()),
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not read {}: {e}", path.display()),
            "reading Studio event metadata failed",
        )
    })?;
    let log: StudioEventLog = serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("studio-events.json is not valid JSON: {e}"),
            format!("malformed Studio event metadata at {}", path.display()),
        )
    })?;
    if log.version != STUDIO_EVENTS_VERSION {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported Studio event metadata version {}", log.version),
            format!(
                "expected studio-events.json version {}",
                STUDIO_EVENTS_VERSION
            ),
        ));
    }
    if log.events.len() > MAX_STUDIO_EVENTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "studio-events.json has too many events: {} (limit: {})",
                log.events.len(),
                MAX_STUDIO_EVENTS
            ),
            "Recording Studio event metadata must be bounded",
        ));
    }
    for event in &log.events {
        validate_studio_event(event)?;
    }
    Ok(log)
}

pub(crate) fn apply_studio_events_to_plan(
    plan: &mut record_core::EditPlan,
    webcam_source: Option<String>,
    camera_clock: Option<record_core::CameraClockRange>,
    log: &StudioEventLog,
) -> Result<usize, CutError> {
    let mut studio_timeline: Vec<(u64, u64, record_core::WebcamKeyframe)> = log
        .events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            studio_event_to_webcam_keyframe(event).map(|result| {
                result.map(|keyframe| {
                    (
                        keyframe.t_ms,
                        event.logical_ts.unwrap_or(index as u64),
                        keyframe,
                    )
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Playback still follows recording time, while equal timestamps retain the
    // exact order in which the recorder accepted the overlapping requests.
    studio_timeline.sort_by_key(|(t_ms, logical_ts, _)| (*t_ms, *logical_ts));

    if let Some(background) = log
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.source == "background" && event.kind == "style")
        .max_by_key(|(index, event)| (event.logical_ts.unwrap_or(*index as u64), *index))
        .map(|(_, event)| event)
        .and_then(|event| event.background.as_deref())
    {
        plan.background = studio_background_to_record_background(background)?;
        if background == "none" {
            // "None" is full-bleed source footage, not a transparent card
            // floating over transparent/black output margins.
            plan.frame.enabled = false;
            plan.frame.padding = 0.0;
            plan.frame.corner_radius = 0.0;
            plan.frame.shadow = None;
        }
    }

    let existing_source = plan.webcam.as_ref().map(|wc| wc.source.clone());
    let Some(source) = webcam_source.or(existing_source) else {
        plan.validate().map_err(crate::screen_record::record_err)?;
        return Ok(0);
    };
    let base_size = studio_timeline
        .iter()
        .find_map(|(_, _, key)| key.size)
        .or_else(|| {
            plan.webcam
                .as_ref()
                .and_then(|wc| wc.timeline.iter().find_map(|key| key.size))
        })
        .or_else(|| plan.webcam.as_ref().map(|wc| wc.size))
        .unwrap_or(0.22);
    let base_shape = studio_timeline
        .iter()
        .find_map(|(_, _, key)| key.shape.clone())
        .or_else(|| {
            plan.webcam
                .as_ref()
                .and_then(|wc| wc.timeline.iter().find_map(|key| key.shape.clone()))
        })
        .or_else(|| plan.webcam.as_ref().map(|wc| wc.shape))
        .unwrap_or(record_core::WebcamShape::Circle);
    // A scene replay is the base timeline. Studio controls are explicit user
    // edits made after recording starts, including the initial configuration
    // event at t=0. Keep both histories: a later scene activation changes the
    // camera from that point, while a later Studio edit changes it again.
    //
    // Scene events have CaptureClock time plus an internal sequence; Studio
    // events have UI elapsed time plus their own server journal sequence. The
    // editable camera timeline retains only millisecond boundaries, so the two
    // logs have no cross-journal ordering at an equal millisecond. Resolve that
    // deliberate tie in favor of the explicit Studio edit. It preserves the
    // start-time configuration shown in the UI and gives the direct camera
    // control a deterministic override when the clocks quantize together.
    let existing_timeline = plan
        .webcam
        .as_ref()
        .map(|webcam| webcam.timeline.clone())
        .unwrap_or_default();
    let studio_event_count = studio_timeline.len();
    let mut timeline = existing_timeline
        .into_iter()
        .enumerate()
        .map(|(index, keyframe)| (keyframe.t_ms, index as u64, keyframe))
        .collect::<Vec<_>>();
    let scene_key_count = timeline.len() as u64;
    timeline.extend(
        studio_timeline
            .into_iter()
            .map(|(t_ms, logical_ts, keyframe)| {
                (t_ms, scene_key_count.saturating_add(logical_ts), keyframe)
            }),
    );
    timeline.sort_by_key(|(t_ms, accepted_order, _)| (*t_ms, *accepted_order));
    let timeline = timeline
        .into_iter()
        .map(|(_, _, keyframe)| keyframe)
        .collect::<Vec<_>>();

    plan.webcam = Some(record_core::WebcamOverlay {
        source,
        shape: base_shape,
        anchor: plan
            .webcam
            .as_ref()
            .map(|wc| wc.anchor)
            .unwrap_or(record_core::Anchor::BottomRight),
        margin: plan.webcam.as_ref().map(|wc| wc.margin).unwrap_or(0.04),
        size: base_size,
        camera_clock: camera_clock.or_else(|| plan.webcam.as_ref().and_then(|wc| wc.camera_clock)),
        timeline,
    });
    plan.validate().map_err(crate::screen_record::record_err)?;
    Ok(studio_event_count)
}

fn studio_event_to_webcam_keyframe(
    event: &StudioEvent,
) -> Option<Result<record_core::WebcamKeyframe, CutError>> {
    if event.source != "camera" {
        return None;
    }
    Some(match event.kind.as_str() {
        "visibility" => Ok(record_core::WebcamKeyframe {
            t_ms: event.t_ms,
            visible: event.visible,
            x: None,
            y: None,
            size: None,
            shape: None,
        }),
        "transform" => {
            let shape = match event.shape.as_deref() {
                Some(shape) => match studio_shape_to_record_shape(shape, event.radius) {
                    Ok(shape) => Some(shape),
                    Err(err) => return Some(Err(err)),
                },
                None => None,
            };
            Ok(record_core::WebcamKeyframe {
                t_ms: event.t_ms,
                visible: event.visible,
                x: event.x,
                y: event.y,
                size: event.size,
                shape,
            })
        }
        _ => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported camera Studio event kind '{}'", event.kind),
            "only camera visibility and transform events can patch the EditPlan",
        )),
    })
}

fn studio_shape_to_record_shape(
    shape: &str,
    radius: Option<f64>,
) -> Result<record_core::WebcamShape, CutError> {
    match shape {
        "circle" => Ok(record_core::WebcamShape::Circle),
        "rounded_rect" => Ok(record_core::WebcamShape::RoundedRect {
            radius: radius.unwrap_or(18.0),
        }),
        _ => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported camera shape '{shape}'"),
            "shape must be circle or rounded_rect",
        )),
    }
}

fn studio_background_to_record_background(
    background: &str,
) -> Result<record_core::Background, CutError> {
    match background {
        "gradient" => Ok(record_core::Background::default()),
        "solid" => Ok(record_core::Background::Solid {
            color: record_core::Rgba::rgb(18, 20, 28),
        }),
        "none" => Ok(record_core::Background::Transparent),
        "blur_screen" => Ok(record_core::Background::BlurScreen { sigma: 8.0 }),
        _ => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported Studio background '{background}'"),
            "background must be gradient, solid, blur_screen, or none",
        )),
    }
}

pub(crate) fn validate_studio_event(event: &StudioEvent) -> Result<(), CutError> {
    let invalid = |field: &str, cause: &str| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("Studio event {field} is invalid"),
            cause.to_string(),
        )
    };

    if event.t_ms > MAX_STUDIO_EVENT_T_MS {
        return Err(invalid("t_ms", "t_ms must be at most 24 hours"));
    }
    match (event.source.as_str(), event.kind.as_str()) {
        ("camera", "visibility") => {
            if event.visible.is_none() {
                return Err(invalid(
                    "visible",
                    "camera visibility events require visible:true|false",
                ));
            }
            validate_no_camera_transform(event)?;
        }
        ("camera", "transform") => {
            let has_transform = event.x.is_some()
                || event.y.is_some()
                || event.size.is_some()
                || event.shape.is_some()
                || event.radius.is_some();
            if !has_transform {
                return Err(invalid(
                    "transform",
                    "camera transform events require x, y, size, shape, or radius",
                ));
            }
            validate_camera_transform(event)?;
        }
        ("recording", "marker") => {
            if let Some(label) = &event.label {
                if label.chars().count() > 120 {
                    return Err(invalid(
                        "label",
                        "marker label must be at most 120 characters",
                    ));
                }
            }
            validate_no_camera_transform(event)?;
        }
        ("background", "style") => {
            let Some(background) = event.background.as_deref() else {
                return Err(invalid(
                    "background",
                    "background style events require background",
                ));
            };
            let _ = studio_background_to_record_background(background)?;
            validate_no_camera_transform(event)?;
        }
        _ => {
            return Err(invalid(
                "source",
                "supported events are camera visibility, camera transform, background style, and recording marker",
            ));
        }
    }
    Ok(())
}

fn validate_no_camera_transform(event: &StudioEvent) -> Result<(), CutError> {
    if event.x.is_none()
        && event.y.is_none()
        && event.size.is_none()
        && event.shape.is_none()
        && event.radius.is_none()
    {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "Studio event transform fields are invalid",
        "only camera transform events may include x, y, size, shape, or radius",
    ))
}

fn validate_camera_transform(event: &StudioEvent) -> Result<(), CutError> {
    let invalid = |field: &str, cause: &str| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("Studio event {field} is invalid"),
            cause.to_string(),
        )
    };
    if let Some(x) = event.x {
        if !(x.is_finite() && (0.0..=1.0).contains(&x)) {
            return Err(invalid("x", "x must be a normalized fraction in [0, 1]"));
        }
    }
    if let Some(y) = event.y {
        if !(y.is_finite() && (0.0..=1.0).contains(&y)) {
            return Err(invalid("y", "y must be a normalized fraction in [0, 1]"));
        }
    }
    if let Some(size) = event.size {
        if !(size.is_finite() && size > 0.0 && size <= 1.0) {
            return Err(invalid(
                "size",
                "size must be a normalized fraction in (0, 1]",
            ));
        }
    }
    if let Some(shape) = &event.shape {
        if shape != "circle" && shape != "rounded_rect" {
            return Err(invalid("shape", "shape must be circle or rounded_rect"));
        }
    }
    if let Some(radius) = event.radius {
        if !(radius.is_finite() && radius >= 0.0) {
            return Err(invalid("radius", "radius must be finite and >= 0"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_none_removes_the_backdrop_and_decorative_frame() {
        let mut plan = record_core::EditPlan::empty(320, 180, 1_000, 30.0);
        let log = StudioEventLog {
            version: STUDIO_EVENTS_VERSION,
            events: vec![StudioEvent {
                logical_ts: Some(1),
                t_ms: 0,
                source: "background".into(),
                kind: "style".into(),
                visible: None,
                x: None,
                y: None,
                size: None,
                shape: None,
                radius: None,
                label: None,
                background: Some("none".into()),
            }],
        };
        apply_studio_events_to_plan(&mut plan, None, None, &log).unwrap();
        assert!(matches!(
            plan.background,
            record_core::Background::Transparent
        ));
        assert!(!plan.frame.enabled);
        assert_eq!(plan.frame.padding, 0.0);
        assert_eq!(plan.frame.corner_radius, 0.0);
        assert!(plan.frame.shadow.is_none());
        assert!(matches!(
            studio_background_to_record_background("solid").unwrap(),
            record_core::Background::Solid { .. }
        ));
    }

    #[test]
    fn studio_keys_merge_after_scene_keys_without_erasing_later_scene_changes() {
        let mut plan = record_core::EditPlan::empty(320, 180, 2_000, 30.0);
        // This is the plan just produced by Recording Scenes: its initial
        // composition is followed by a live scene activation at 1s.
        plan.webcam = Some(record_core::WebcamOverlay {
            source: "scene-camera.mp4".into(),
            shape: record_core::WebcamShape::Circle,
            anchor: record_core::Anchor::TopRight,
            margin: 0.04,
            size: 0.25,
            camera_clock: None,
            timeline: vec![
                record_core::WebcamKeyframe {
                    t_ms: 0,
                    visible: Some(true),
                    x: Some(0.70),
                    y: Some(0.04),
                    size: Some(0.25),
                    shape: Some(record_core::WebcamShape::Circle),
                },
                record_core::WebcamKeyframe {
                    t_ms: 1_000,
                    visible: Some(true),
                    x: Some(0.04),
                    y: Some(0.04),
                    size: Some(0.20),
                    shape: Some(record_core::WebcamShape::RoundedRect { radius: 18.0 }),
                },
            ],
        });
        let log = StudioEventLog {
            version: STUDIO_EVENTS_VERSION,
            events: vec![
                // The initial Studio configuration is accepted after the
                // capture-start scene snapshot, so it wins at t=0.
                StudioEvent {
                    logical_ts: Some(1),
                    t_ms: 0,
                    source: "camera".into(),
                    kind: "transform".into(),
                    visible: None,
                    x: Some(0.10),
                    y: Some(0.20),
                    size: Some(0.30),
                    shape: Some("circle".into()),
                    radius: None,
                    label: None,
                    background: None,
                },
                // When UI and CaptureClock quantize a later scene action to
                // the same millisecond, the explicit Studio control wins.
                StudioEvent {
                    logical_ts: Some(2),
                    t_ms: 1_000,
                    source: "camera".into(),
                    kind: "transform".into(),
                    visible: None,
                    x: Some(0.80),
                    y: Some(0.20),
                    size: Some(0.28),
                    shape: Some("circle".into()),
                    radius: None,
                    label: None,
                    background: None,
                },
                // A manual live edit after the later scene activation wins
                // from its own timestamp without deleting the scene history.
                StudioEvent {
                    logical_ts: Some(3),
                    t_ms: 1_500,
                    source: "camera".into(),
                    kind: "transform".into(),
                    visible: None,
                    x: Some(0.60),
                    y: Some(0.50),
                    size: Some(0.35),
                    shape: Some("rounded_rect".into()),
                    radius: Some(12.0),
                    label: None,
                    background: None,
                },
            ],
        };

        assert_eq!(
            apply_studio_events_to_plan(&mut plan, None, None, &log).unwrap(),
            3
        );
        let timeline = &plan.webcam.as_ref().unwrap().timeline;
        assert_eq!(timeline.len(), 5);
        assert_eq!(timeline[0].t_ms, 0);
        assert_eq!(timeline[0].x, Some(0.70));
        assert_eq!(timeline[1].t_ms, 0);
        assert_eq!(timeline[1].x, Some(0.10));
        assert_eq!(timeline[2].t_ms, 1_000);
        assert_eq!(timeline[2].x, Some(0.04));
        assert_eq!(timeline[3].t_ms, 1_000);
        assert_eq!(timeline[3].x, Some(0.80));
        assert_eq!(timeline[4].t_ms, 1_500);
        assert_eq!(timeline[4].x, Some(0.60));
        assert!(plan.validate().is_ok());
    }
}
