//! Durable recording provenance and replay-only take edits. No recorder dependency.
use crate::{
    error::{codes, CutError},
    types::{Asset, Clip, Project},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub(crate) mod camera;
pub use camera::camera_look_for_clip;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomOrigin {
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub scale: f64,
    pub cx: f64,
    pub cy: f64,
    pub ramp_in_ms: u64,
    pub ramp_out_ms: u64,
    pub origin: ZoomOrigin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraLook {
    pub transform: crate::types::ClipTransform,
    pub keyframes: Vec<crate::types::Keyframe>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingTake {
    pub id: String,
    pub capture_folder: String,
    pub source: String,
    pub events: String,
    pub pending_plan: String,
    pub applied_plan: String,
    pub clip_ids: Vec<String>,
    pub asset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_artifact: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_clip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_look: Option<CameraLook>,
    pub segments: Vec<ZoomSegment>,
}

impl RecordingTake {
    pub fn stale(&self) -> bool {
        self.pending_plan != self.applied_plan
    }
    /// Asset identity also finds split halves, copies, and nested clips. Explicit
    /// ids alone would lose newly allocated split ids. Replaced clips detach.
    pub fn owns(&self, clip: &crate::types::MediaClip) -> bool {
        clip.asset == self.asset
    }
}

pub fn take_for_clip<'a>(project: &'a Project, clip: &str) -> Option<&'a RecordingTake> {
    let (track, index) = project.find_clip(clip)?;
    let Clip::Media(media) = &project.track(track)?.clips[index] else {
        return None;
    };
    project.recordings.iter().find(|take| take.owns(media))
}

fn bad(message: &str) -> CutError {
    CutError::new(codes::INVALID_ARGS, "invalid recording edit", message)
}

/// The history operations this module applies (the store dispatches them here).
pub(crate) fn is_op(verb: &str) -> bool {
    matches!(
        verb,
        "screen_record.take_link"
            | "screen_record.zoom_set"
            | "screen_record.take_refresh"
            | "screen_record.camera_reset"
    )
}

pub(crate) fn apply(project: &mut Project, verb: &str, args: &Value) -> Result<(), CutError> {
    apply_inner(project, verb, args, true)
}

/// Describe a reset using the same transform effect fields as edit.transform,
/// plus the replaced keyframes. Recording provenance ops retain their effects.
pub(crate) fn effects(
    before: &Project,
    after: &Project,
    verb: &str,
    args: &Value,
) -> Vec<crate::OpEffect> {
    if verb != "screen_record.camera_reset" {
        return vec![];
    }
    // Called only after apply succeeded on a clone, so both camera slices
    // exist; an effect is a description, so anything unexpected records none.
    let Some(clip) = args["clip"].as_str() else {
        return vec![];
    };
    let (Some((track, index)), Some((old_track, old_index))) =
        (after.find_clip(clip), before.find_clip(clip))
    else {
        return vec![];
    };
    let (Some(Clip::Media(new)), Some(Clip::Media(old))) = (
        after.track(track).and_then(|t| t.clips.get(index)),
        before.track(old_track).and_then(|t| t.clips.get(old_index)),
    ) else {
        return vec![];
    };
    vec![crate::OpEffect {
        track: Some(track.to_owned()),
        detail: serde_json::json!({"clip":clip,"old_transform":old.transform,"new_transform":new.transform,
            "shape":new.transform.as_ref().map(|t| t.shape).unwrap_or_default(),
            "old_keyframes":old.keyframes,"keyframes":new.keyframes})
            .as_object().unwrap().clone(),
    }]
}

// Existing durable journals may contain the former unrestricted camera look.
// Only materialization of those saved ops uses this route.
pub(crate) fn apply_saved(project: &mut Project, verb: &str, args: &Value) -> Result<(), CutError> {
    apply_inner(project, verb, args, false)
}

fn apply_inner(
    project: &mut Project,
    verb: &str,
    args: &Value,
    enforce_bound: bool,
) -> Result<(), CutError> {
    match verb {
        "screen_record.camera_reset" => {
            let clip = args["clip"]
                .as_str()
                .ok_or_else(|| bad("camera clip is missing"))?;
            let look = if enforce_bound {
                camera_look_for_clip(project, clip)?
            } else {
                camera::camera_look_for_clip_legacy(project, clip)?
            };
            let (track, index) = project
                .find_clip(clip)
                .ok_or_else(|| bad("camera clip is missing"))?;
            let track = track.to_owned();
            let Clip::Media(media) = &mut project.track_mut(&track).unwrap().clips[index] else {
                return Err(bad("camera must be media"));
            };
            media.transform = Some(look.transform);
            media.keyframes = look.keyframes;
        }
        "screen_record.take_link" => {
            let take: RecordingTake = serde_json::from_value(args["recording"].clone())?;
            if take.id.is_empty()
                || project.recordings.iter().any(|t| {
                    t.id == take.id
                        || t.asset == take.asset
                        || (take.camera_look.is_some()
                            && take.camera_asset.is_some()
                            && t.camera_asset == take.camera_asset)
                })
            {
                return Err(bad("take id and asset must be unique"));
            }
            if !project.assets.contains_key(&take.asset) {
                return Err(bad("take asset is missing"));
            }
            validate_segments(&take.segments)?;
            if enforce_bound {
                camera::validate_take_camera(project, &take)?;
            } else {
                camera::validate_take_camera_legacy(project, &take)?;
            }
            project.recordings.push(take);
        }
        "screen_record.zoom_set" => {
            let segments: Vec<ZoomSegment> = serde_json::from_value(args["segments"].clone())?;
            validate_segments(&segments)?;
            let plan = args["pending_plan"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad("pending plan is missing"))?;
            let take = project
                .recordings
                .iter_mut()
                .find(|t| Some(t.id.as_str()) == args["take"].as_str())
                .ok_or_else(|| bad("take is missing"))?;
            take.segments = segments;
            take.pending_plan = plan.into();
        }
        "screen_record.take_refresh" => {
            let id = args["take"]
                .as_str()
                .ok_or_else(|| bad("take is missing"))?;
            let index = project
                .recordings
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(|| bad("take is missing"))?;
            let old = project.recordings[index].asset.clone();
            if args["applied_plan"].as_str()
                != Some(project.recordings[index].pending_plan.as_str())
            {
                return Err(bad(
                    "take changed while the bake was running; Update video again",
                ));
            }
            let asset: Asset = serde_json::from_value(args["asset"].clone())?;
            let asset_id = args["asset_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad("asset id is missing"))?;
            if project.assets.get(asset_id).is_some_and(|a| a != &asset) {
                return Err(bad("asset id already exists"));
            }
            project.assets.insert(asset_id.into(), asset);
            fn swap(tracks: &mut [crate::types::Track], old: &str, new: &str) {
                for track in tracks {
                    for clip in &mut track.clips {
                        if let Clip::Media(m) = clip {
                            if m.asset == old {
                                m.asset = new.into();
                            }
                        }
                    }
                }
            }
            swap(&mut project.tracks, &old, asset_id);
            for nest in &mut project.nests {
                swap(&mut nest.tracks, &old, asset_id);
            }
            for sequence in &mut project.sequences {
                swap(&mut sequence.tracks, &old, asset_id);
                for nest in &mut sequence.nests {
                    swap(&mut nest.tracks, &old, asset_id);
                }
            }
            project.recordings[index].asset = asset_id.into();
            project.recordings[index].applied_plan = project.recordings[index].pending_plan.clone();
        }
        _ => return Err(bad("unsupported recording operation")),
    }
    Ok(())
}

pub fn validate_segments(segments: &[ZoomSegment]) -> Result<(), CutError> {
    let mut end = 0;
    let mut ids = std::collections::BTreeSet::new();
    for s in segments {
        if s.id.is_empty() || !ids.insert(&s.id) || s.start_ms < end || s.start_ms >= s.end_ms {
            return Err(bad(
                "zoom ids must be unique and segments ordered without overlap",
            ));
        }
        let length = s.end_ms - s.start_ms;
        if !s.scale.is_finite()
            || !(1.0..=8.0).contains(&s.scale)
            || s.ramp_in_ms > length
            || s.ramp_out_ms > length - s.ramp_in_ms
        {
            return Err(bad("invalid zoom scale or ramps"));
        }
        let half = 0.5 / s.scale;
        if !s.cx.is_finite()
            || !s.cy.is_finite()
            || !(half..=1.0 - half).contains(&s.cx)
            || !(half..=1.0 - half).contains(&s.cy)
        {
            return Err(bad("zoom target is outside the frame"));
        }
        end = s.end_ms;
    }
    Ok(())
}
