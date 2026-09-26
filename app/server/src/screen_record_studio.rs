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

use crate::screen_record_studio_camera_timeline::{merge_camera_timeline, CameraTimelineEdit};

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
    let studio_edits: Vec<CameraTimelineEdit> = log
        .events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            let order = event.logical_ts.unwrap_or(index as u64);
            if event.source == "camera" && event.kind == "reset" {
                Some(Ok(CameraTimelineEdit::Reset {
                    order,
                    t_ms: event.t_ms,
                }))
            } else {
                studio_event_to_webcam_keyframe(event)
                    .map(|result| result.map(|key| CameraTimelineEdit::Transform { order, key }))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

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
    let base_size = plan.webcam.as_ref().map(|wc| wc.size).unwrap_or(0.22);
    let base_shape = plan
        .webcam
        .as_ref()
        .map(|wc| wc.shape)
        .unwrap_or(record_core::WebcamShape::Circle);
    let mut webcam = record_core::WebcamOverlay {
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
        timeline: Vec::new(),
    };
    let base = webcam.placement_at(0, plan.source_w, plan.source_h);
    let static_layout = record_core::WebcamKeyframe {
        t_ms: 0,
        visible: None,
        x: Some(base.x),
        y: Some(base.y),
        size: Some(base.size),
        shape: Some(base.shape),
    };
    let scene_keys = plan
        .webcam
        .as_ref()
        .map(|wc| wc.timeline.clone())
        .unwrap_or_default();
    let studio_event_count = studio_edits.len();
    webcam.timeline = merge_camera_timeline(scene_keys, studio_edits, static_layout);
    plan.webcam = Some(webcam);
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
        ("camera", "reset") => {
            if event.visible.is_some()
                || event.x.is_some()
                || event.y.is_some()
                || event.size.is_some()
                || event.shape.is_some()
                || event.radius.is_some()
                || event.label.is_some()
                || event.background.is_some()
            {
                return Err(invalid(
                    "reset",
                    "camera layout reset has no payload fields",
                ));
            }
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
                "supported events are camera visibility, transform, reset, background style, and recording marker",
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
    fn studio_layout_survives_scene_switch_until_explicit_camera_reset() {
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
                record_core::WebcamKeyframe {
                    t_ms: 1_800,
                    visible: Some(true),
                    x: Some(0.70),
                    y: Some(0.70),
                    size: Some(0.25),
                    shape: Some(record_core::WebcamShape::Circle),
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
                    x: Some(0.04),
                    y: Some(0.46),
                    size: Some(0.50),
                    shape: Some("circle".into()),
                    radius: None,
                    label: None,
                    background: None,
                },
                // A later direct Studio choice replaces the carried layout.
                StudioEvent {
                    logical_ts: Some(2),
                    t_ms: 1_200,
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
                StudioEvent {
                    logical_ts: Some(4),
                    t_ms: 1_600,
                    source: "camera".into(),
                    kind: "reset".into(),
                    visible: None,
                    x: None,
                    y: None,
                    size: None,
                    shape: None,
                    radius: None,
                    label: None,
                    background: None,
                },
            ],
        };

        assert_eq!(
            apply_studio_events_to_plan(&mut plan, None, None, &log).unwrap(),
            4
        );
        let timeline = &plan.webcam.as_ref().unwrap().timeline;
        assert_eq!(timeline.len(), 7);
        assert_eq!(timeline[0].t_ms, 0);
        assert_eq!(timeline[0].x, Some(0.70));
        assert_eq!(timeline[1].t_ms, 0);
        assert_eq!(timeline[1].x, Some(0.04));
        assert_eq!(timeline[2].t_ms, 1_000);
        assert_eq!(timeline[2].x, Some(0.04));
        assert_eq!(timeline[2].y, Some(0.46));
        assert_eq!(timeline[2].size, Some(0.50));
        assert_eq!(timeline[2].shape, Some(record_core::WebcamShape::Circle));
        assert_eq!(timeline[3].t_ms, 1_200);
        assert_eq!(timeline[3].x, Some(0.80));
        assert_eq!(timeline[4].t_ms, 1_500);
        assert_eq!(timeline[4].x, Some(0.60));
        assert_eq!(timeline[5].t_ms, 1_600);
        assert_eq!(timeline[5].x, Some(0.04));
        assert_eq!(timeline[5].size, Some(0.20));
        assert_eq!(
            timeline[5].shape,
            Some(record_core::WebcamShape::RoundedRect { radius: 18.0 })
        );
        assert_eq!(timeline[6].t_ms, 1_800);
        assert_eq!(timeline[6].x, Some(0.70));
        assert_eq!(
            plan.webcam
                .as_ref()
                .unwrap()
                .placement_at(1_250, 320, 180)
                .x,
            0.80
        );
        let retained = plan.webcam.as_ref().unwrap().placement_at(1_100, 320, 180);
        assert_eq!((retained.x, retained.y, retained.size), (0.04, 0.46, 0.50));
        assert_eq!(retained.shape, record_core::WebcamShape::Circle);
        assert_eq!(
            plan.webcam
                .as_ref()
                .unwrap()
                .placement_at(1_550, 320, 180)
                .x,
            0.60
        );
        assert_eq!(
            plan.webcam
                .as_ref()
                .unwrap()
                .placement_at(1_650, 320, 180)
                .x,
            0.04
        );
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn camera_layout_reset_rejects_payload_fields() {
        let mut event = StudioEvent {
            logical_ts: None,
            t_ms: 50,
            source: "camera".into(),
            kind: "reset".into(),
            visible: None,
            x: None,
            y: None,
            size: None,
            shape: None,
            radius: None,
            label: None,
            background: None,
        };
        assert!(validate_studio_event(&event).is_ok());
        event.x = Some(0.2);
        assert!(validate_studio_event(&event).is_err());
        event.x = None;
        event.visible = Some(false);
        assert!(validate_studio_event(&event).is_err());
    }

    #[test]
    fn camera_layout_reset_without_scenes_restores_static_camera_layout() {
        let mut plan = record_core::EditPlan::empty(320, 180, 1_000, 30.0);
        let log = StudioEventLog {
            version: STUDIO_EVENTS_VERSION,
            events: vec![
                StudioEvent {
                    logical_ts: Some(1),
                    t_ms: 100,
                    source: "camera".into(),
                    kind: "transform".into(),
                    visible: None,
                    x: Some(0.04),
                    y: Some(0.04),
                    size: Some(0.35),
                    shape: Some("rounded_rect".into()),
                    radius: None,
                    label: None,
                    background: None,
                },
                StudioEvent {
                    logical_ts: Some(2),
                    t_ms: 200,
                    source: "camera".into(),
                    kind: "reset".into(),
                    visible: None,
                    x: None,
                    y: None,
                    size: None,
                    shape: None,
                    radius: None,
                    label: None,
                    background: None,
                },
            ],
        };
        apply_studio_events_to_plan(&mut plan, Some("camera.mp4".into()), None, &log).unwrap();
        let webcam = plan.webcam.as_ref().unwrap();
        let selected = webcam.placement_at(150, 320, 180);
        assert_eq!((selected.x, selected.y, selected.size), (0.04, 0.04, 0.35));
        assert!(matches!(
            selected.shape,
            record_core::WebcamShape::RoundedRect { .. }
        ));
        let reset = webcam.placement_at(250, 320, 180);
        assert!(reset.x > 0.8 && reset.y > 0.7);
        assert_eq!(reset.size, 0.22);
        assert_eq!(reset.shape, record_core::WebcamShape::Circle);
    }
}
