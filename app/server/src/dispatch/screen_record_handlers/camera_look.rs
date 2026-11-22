//! Convert the recorder's height-relative bubble into an editable square overlay.
use crate::{events::Event, state::AppState};
use cut_core::{
    recording::{CameraLook, RecordingTake},
    types::{ClipTransform, Keyframe, KfInterp, KfParam, KfPoint, OverlayRing, OverlayShape},
    Actor, CutError, VerbResult,
};
use record_core::EditPlan;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

// The recorder admits at most 50,000 Studio events. A camera timeline cannot
// legitimately contain more independent changes than that recording budget.
const MAX_CAMERA_TIMELINE_KEYS: usize = 50_000;

fn validate_camera_timeline(plan: &EditPlan) -> Result<(), CutError> {
    if plan
        .webcam
        .as_ref()
        .is_some_and(|webcam| webcam.timeline.len() > MAX_CAMERA_TIMELINE_KEYS)
    {
        return Err(crate::screen_record::take_edit::bad(
            "camera timeline has too many changes (limit: 50000)",
        ));
    }
    plan.validate()
        .map_err(|error| crate::screen_record::take_edit::bad(&error.to_string()))
}

#[cfg(test)]
mod tests {
    fn camera_plan() -> record_core::EditPlan {
        let mut plan = record_core::EditPlan::empty(640, 360, 60_000, 30.0);
        plan.webcam = Some(record_core::WebcamOverlay {
            source: "camera.mp4".into(),
            shape: record_core::WebcamShape::Circle,
            anchor: record_core::Anchor::BottomRight,
            margin: 0.05,
            size: 0.3,
            camera_clock: None,
            timeline: vec![],
        });
        plan
    }

    #[test]
    fn rounded_camera_radius_converts_recorded_pixels_to_bubble_fraction() {
        let mut plan = record_core::EditPlan::empty(640, 360, 3000, 30.0);
        plan.webcam = Some(record_core::WebcamOverlay {
            source: "camera.mp4".into(),
            shape: record_core::WebcamShape::RoundedRect { radius: 18.0 },
            anchor: record_core::Anchor::BottomRight,
            margin: 0.05,
            size: 0.3,
            camera_clock: None,
            timeline: vec![],
        });
        let look = super::recorded(&plan, 500).unwrap().unwrap();
        assert_eq!(look.transform.shape, cut_core::types::OverlayShape::Rounded);
        assert!((look.transform.radius.unwrap() - 18.0 / 108.0).abs() < 1e-9);
    }

    #[test]
    fn occupied_screen_plan_refuses_oversized_cache_before_reading() {
        use sha2::{Digest, Sha256};
        let project = tempfile::tempdir().unwrap();
        let cache = crate::screen_record::screen_record_cache_dir(project.path()).unwrap();
        let plan = camera_plan();
        let mut screen = plan.clone();
        screen.webcam = None;
        let bytes = serde_json::to_vec_pretty(&screen).unwrap();
        let occupied = cache.join(format!("plan-{:x}.json", Sha256::digest(&bytes)));
        std::fs::File::create(&occupied)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let error = super::screen_plan(&cache, &cache.join("original.json"), &plan).unwrap_err();
        assert!(error.message.contains("length differs"), "{error}");
    }

    #[test]
    fn screen_plan_reuses_identical_plain_cache_bytes() {
        let project = tempfile::tempdir().unwrap();
        let cache = crate::screen_record::screen_record_cache_dir(project.path()).unwrap();
        let plan = camera_plan();
        let original = cache.join("original.json");
        let first = super::screen_plan(&cache, &original, &plan).unwrap();
        let first_bytes = std::fs::read(&first).unwrap();
        let second = super::screen_plan(&cache, &original, &plan).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(second).unwrap(), first_bytes);
    }

    #[test]
    fn recorded_camera_refuses_more_keys_than_recorder_event_budget() {
        let mut plan = camera_plan();
        plan.webcam.as_mut().unwrap().timeline = (0..=super::MAX_CAMERA_TIMELINE_KEYS)
            .map(|index| record_core::WebcamKeyframe {
                t_ms: index as u64,
                visible: Some(index % 2 == 0),
                x: None,
                y: None,
                size: None,
                shape: None,
            })
            .collect();
        let error = super::recorded(&plan, 0).unwrap_err();
        assert!(error.message.contains("too many changes"), "{error}");
    }

    #[test]
    fn recorded_camera_merges_same_millisecond_before_emitting_channels() {
        let mut plan = camera_plan();
        plan.webcam.as_mut().unwrap().timeline = vec![
            record_core::WebcamKeyframe {
                t_ms: 100,
                visible: Some(false),
                x: Some(0.1),
                y: None,
                size: None,
                shape: None,
            },
            record_core::WebcamKeyframe {
                t_ms: 100,
                visible: Some(true),
                x: Some(0.2),
                y: None,
                size: None,
                shape: None,
            },
        ];
        let look = super::recorded(&plan, 0).unwrap().unwrap();
        assert!(look
            .keyframes
            .iter()
            .all(|channel| channel.points.len() == 2));
        assert!(look
            .keyframes
            .iter()
            .all(|channel| channel.param != cut_core::types::KfParam::Opacity));
        assert_eq!(look.keyframes[0].points[1].t_ms, 100);
        assert_eq!(look.keyframes[0].points[1].value, 0.2);
    }

    #[test]
    fn recorded_camera_refuses_changed_channel_over_track_limit() {
        let mut plan = camera_plan();
        plan.webcam.as_mut().unwrap().timeline = (1
            ..=cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|index| record_core::WebcamKeyframe {
                t_ms: index as u64,
                visible: Some(index % 2 == 0),
                x: None,
                y: None,
                size: None,
                shape: None,
            })
            .collect();
        let error = super::recorded(&plan, 0).unwrap_err();
        assert!(error.message.contains("more than 64 changes"), "{error}");
        assert!(error
            .suggested_action
            .unwrap()
            .contains("camera drawn into the video"));
    }
}

pub(crate) fn recorded(plan: &EditPlan, offset: u64) -> Result<Option<CameraLook>, CutError> {
    let Some(webcam) = &plan.webcam else {
        return Ok(None);
    };
    validate_camera_timeline(plan)?;
    let (w, h) = plan
        .checked_output_size()
        .map_err(|e| crate::screen_record::take_edit::bad(&e.to_string()))?;
    let placement = webcam.placement_at(offset, w, h);
    let diameter = (placement.size * h as f64).round().max(2.0);
    let x = (placement.x * w as f64).clamp(0.0, (w as f64 - diameter).max(0.0)) / w as f64;
    let y = (placement.y * h as f64).clamp(0.0, (h as f64 - diameter).max(0.0)) / h as f64;
    let (shape, radius) = match placement.shape {
        record_core::WebcamShape::Circle => (OverlayShape::Circle, None),
        record_core::WebcamShape::RoundedRect { radius } => (
            OverlayShape::Rounded,
            Some((radius / diameter).clamp(0.0, 0.5)),
        ),
    };
    let transform = ClipTransform {
        x,
        y,
        scale: diameter / w as f64,
        opacity: if placement.visible { 1.0 } else { 0.0 },
        shape,
        radius,
        ring: Some(OverlayRing {
            width: 0.02,
            colour: "#ffffffe6".into(),
        }),
    };
    let params = [
        KfParam::PosX,
        KfParam::PosY,
        KfParam::Opacity,
        KfParam::Scale,
    ];
    let mut channels = [x, y, transform.opacity, 1.0].map(|value| vec![KfPoint { t_ms: 0, value }]);
    let mut current = placement;
    let mut keys = webcam
        .timeline
        .iter()
        .filter(|key| key.t_ms > offset)
        .peekable();
    while let Some(first) = keys.next() {
        if first.t_ms >= plan.duration_ms {
            break;
        }
        let t_ms = first.t_ms;
        // placement_at applies every key at a timestamp before returning.
        // Keep that behavior when several recorder events share a millisecond.
        for key in std::iter::once(first)
            .chain(std::iter::from_fn(|| keys.next_if(|key| key.t_ms == t_ms)))
        {
            if let Some(visible) = key.visible {
                current.visible = visible;
            }
            if let Some(x) = key.x {
                current.x = x;
            }
            if let Some(y) = key.y {
                current.y = y;
            }
            if let Some(size) = key.size {
                current.size = size;
            }
            if let Some(shape) = key.shape {
                current.shape = shape;
            }
        }
        let d = (current.size * h as f64).round().max(2.0);
        let values = [
            current.x.clamp(0.0, (1.0 - d / w as f64).max(0.0)),
            current.y.clamp(0.0, (1.0 - d / h as f64).max(0.0)),
            if current.visible { 1.0 } else { 0.0 },
            d / diameter,
        ];
        for (points, value) in channels.iter_mut().zip(values) {
            if points.last().is_some_and(|last| last.value != value) {
                points.push(KfPoint {
                    t_ms: t_ms - offset,
                    value,
                });
                if points.len() > cut_core::keyframe_limit::MAX_KEYFRAME_POINTS {
                    return Err(CutError::new(
                        cut_core::error_codes::INVALID_ARGS,
                        format!(
                            "recorded camera look has more than {} changes in one channel",
                            cut_core::keyframe_limit::MAX_KEYFRAME_POINTS
                        ),
                        "a separate camera look with this many changes cannot be rendered safely",
                    )
                    .with_suggested_action("record again with the camera drawn into the video"));
                }
            }
        }
    }
    let keyframes = params
        .into_iter()
        .zip(channels)
        .filter_map(|(param, points)| {
            (points.len() > 1).then_some(Keyframe {
                lead_in: None,
                param,
                points,
                interp: KfInterp::Hold,
            })
        })
        .collect();
    Ok(Some(CameraLook {
        transform,
        keyframes,
    }))
}

type CameraBake = (
    Option<EditPlan>,
    PathBuf,
    Option<CameraLook>,
    bool,
    Option<String>,
);

pub(crate) fn prepare(
    dir: &Path,
    project: &cut_core::Project,
    cache: &Path,
    path: PathBuf,
    raw: bool,
    mode: Option<&str>,
    camera: Option<(&record_core::CameraArtifact, &Path)>,
) -> Result<CameraBake, CutError> {
    if raw {
        return Ok((None, path, None, false, None));
    }
    let mut plan = crate::screen_record::plan_inputs::load(dir, project, &path)?;
    let separate = camera.is_some() && mode != Some("baked");
    if separate {
        validate_camera_timeline(&plan)?;
    }
    let note = if separate
        && plan.webcam.as_ref().is_some_and(|webcam| {
            let first = webcam
                .placement_at(
                    camera.unwrap().0.clock.first_frame_offset_ms,
                    plan.source_w,
                    plan.source_h,
                )
                .shape;
            webcam
                .timeline
                .iter()
                .any(|key| key.shape.is_some_and(|shape| shape != first))
        }) {
        Some(
            "Camera shape changed while recording; the separate camera keeps its first shape."
                .into(),
        )
    } else {
        None
    };
    let (path, look) = if separate {
        let (artifact, video) = camera.unwrap();
        // Hand-authored plans can omit webcam. Reuse the recorder's standard
        // look rather than placing an unshaped full-frame camera.
        if plan.webcam.is_none() {
            crate::screen_record_studio::apply_studio_events_to_plan(
                &mut plan,
                Some(video.display().to_string()),
                Some(artifact.clock),
                &crate::screen_record_studio::StudioEventLog::default(),
            )?;
        }
        let look = recorded(&plan, artifact.clock.first_frame_offset_ms)?;
        let screen_path = screen_plan(cache, &path, &plan)?;
        (screen_path, look)
    } else {
        (path, None)
    };
    if separate {
        remove_baked_camera(&mut plan);
    }
    Ok((Some(plan), path, look, separate, note))
}

pub(crate) fn plan_to_bake(take: &RecordingTake, mut plan: EditPlan) -> EditPlan {
    if take.camera_look.is_some() {
        remove_baked_camera(&mut plan);
    }
    plan
}

/// Presenter spans only describe the baked webcam, not a screen reframe.
/// Keep screen identities and timer replay, but no camera-only scene state.
fn remove_baked_camera(plan: &mut EditPlan) {
    plan.webcam = None;
    if let Some(scene) = &mut plan.scene_timeline {
        for span in &mut scene.camera {
            span.presenter = None;
        }
        scene.terminal.composition = record_core::SceneComposition::ScreenOnly;
    }
}

/// Publish immutable bubble-free bytes. The ordinary bake key remains the hash
/// of the actual plan bytes; never reuse a baked-camera cache for separate mode.
pub(crate) fn screen_plan(
    cache: &Path,
    original: &Path,
    plan: &EditPlan,
) -> Result<PathBuf, CutError> {
    let mut screen = plan.clone();
    remove_baked_camera(&mut screen);
    if screen == *plan {
        return Ok(original.to_owned());
    }
    let bytes = serde_json::to_vec_pretty(&screen)?;
    use sha2::{Digest, Sha256};
    let path = cache.join(format!("plan-{:x}.json", Sha256::digest(&bytes)));
    if crate::screen_record::cache_output::cache_hit(&path)? {
        let mut file = cut_core::store::open_plain_file(&path).map_err(|_| {
            crate::screen_record::take_edit::bad("screen plan cache is not a plain file")
        })?;
        if file.metadata()?.len() != bytes.len() as u64 {
            return Err(crate::screen_record::take_edit::bad(
                "screen plan cache length differs",
            ));
        }
        let mut equal = true;
        let mut found = [0u8; 8192];
        for expected in bytes.chunks(found.len()) {
            file.read_exact(&mut found[..expected.len()])?;
            equal &= &found[..expected.len()] == expected;
        }
        // Catch a concurrent append after the length check without reading it.
        if file.read(&mut found[..1])? != 0 || !equal {
            return Err(crate::screen_record::take_edit::bad(
                "screen plan cache identity differs",
            ));
        }
    } else {
        crate::screen_record::cache_output::write(&path, &bytes)?;
    }
    Ok(path)
}

pub(crate) async fn reset(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let clip = args["clip"]
        .as_str()
        .ok_or_else(|| crate::screen_record::take_edit::bad("camera clip is required"))?;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    let look = cut_core::recording::camera_look_for_clip(&store.project, clip)?;
    let op = store.apply(
        "screen_record.camera_reset",
        json!({"clip":clip}),
        actor,
        None,
    )?;
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok(
        json!({"clip":clip,"transform":look.transform,"keyframes":look.keyframes}),
    ))
}
