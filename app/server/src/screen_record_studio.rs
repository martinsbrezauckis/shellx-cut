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

pub(crate) const STUDIO_EVENTS_FILENAME: &str = "studio-events.json";
const STUDIO_EVENTS_VERSION: u32 = 1;
const MAX_STUDIO_EVENTS_JSON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STUDIO_EVENTS: usize = 50_000;
const MAX_STUDIO_EVENT_T_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StudioEvent {
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

    let events_path = studio_events_path(&capture_dir);
    let log = append_studio_event(&events_path, a.event)?;
    let last_event = log.events.last().cloned().unwrap_or_else(|| StudioEvent {
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
    capture_dir.join(STUDIO_EVENTS_FILENAME)
}

pub(crate) fn read_studio_events(path: &Path) -> Result<StudioEventLog, CutError> {
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
    log: &StudioEventLog,
) -> Result<usize, CutError> {
    let timeline: Vec<record_core::WebcamKeyframe> = log
        .events
        .iter()
        .filter_map(studio_event_to_webcam_keyframe)
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(background) = log
        .events
        .iter()
        .rev()
        .find(|event| event.source == "background" && event.kind == "style")
        .and_then(|event| event.background.as_deref())
    {
        plan.background = studio_background_to_record_background(background)?;
    }

    let existing_source = plan.webcam.as_ref().map(|wc| wc.source.clone());
    let Some(source) = webcam_source.or(existing_source) else {
        plan.validate().map_err(crate::screen_record::record_err)?;
        return Ok(0);
    };
    let base_size = timeline
        .iter()
        .find_map(|key| key.size)
        .or_else(|| plan.webcam.as_ref().map(|wc| wc.size))
        .unwrap_or(0.22);
    let base_shape = timeline
        .iter()
        .find_map(|key| key.shape)
        .or_else(|| plan.webcam.as_ref().map(|wc| wc.shape))
        .unwrap_or(record_core::WebcamShape::Circle);

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
        timeline,
    });
    plan.validate().map_err(crate::screen_record::record_err)?;
    Ok(plan
        .webcam
        .as_ref()
        .map(|wc| wc.timeline.len())
        .unwrap_or(0))
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
        "solid" | "none" => Ok(record_core::Background::Solid {
            color: record_core::Rgba::rgb(18, 20, 28),
        }),
        "blur_screen" => Ok(record_core::Background::BlurScreen { sigma: 8.0 }),
        _ => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported Studio background '{background}'"),
            "background must be gradient, solid, blur_screen, or none",
        )),
    }
}

fn append_studio_event(path: &Path, event: StudioEvent) -> Result<StudioEventLog, CutError> {
    let mut log = read_studio_events(path)?;
    if log.events.len() >= MAX_STUDIO_EVENTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("Studio event limit reached ({MAX_STUDIO_EVENTS})"),
            "Recording Studio event metadata must be bounded",
        ));
    }
    if let Some(last) = log.events.last() {
        if event.t_ms < last.t_ms {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "Studio event t_ms is earlier than the previous event",
                "Studio events must be appended in recording-time order",
            ));
        }
    }
    log.events.push(event);
    let bytes = serde_json::to_vec_pretty(&log).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not serialize Studio events: {e}"),
            "Studio event metadata serialization failed",
        )
    })?;
    std::fs::write(path, bytes).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not write {}: {e}", path.display()),
            "writing Studio event metadata failed",
        )
    })?;
    Ok(log)
}

fn validate_studio_event(event: &StudioEvent) -> Result<(), CutError> {
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
