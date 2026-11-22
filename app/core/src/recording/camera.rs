use super::{bad, CameraLook, RecordingTake};
use crate::{
    types::{Clip, ClipTransform, KfParam, KfPoint, TrackKind},
    CutError, Project,
};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

/// A hand placement replaces recorded movement, but not size or visibility.
pub(crate) fn hand_place(project: &mut Project, clip: &str, next: &ClipTransform) {
    let Some((track, index)) = project.find_clip(clip) else {
        return;
    };
    let Clip::Media(media) = &project.track(track).unwrap().clips[index] else {
        return;
    };
    let (x, y) = media
        .transform
        .as_ref()
        .map(|t| (t.x, t.y))
        .unwrap_or((0.0, 0.0));
    if (x == next.x && y == next.y)
        || !project.recordings.iter().any(|take| {
            take.camera_look.is_some() && take.camera_asset.as_deref() == Some(media.asset.as_str())
        })
    {
        return;
    }
    let track = track.to_owned();
    if let Clip::Media(media) = &mut project.track_mut(&track).unwrap().clips[index] {
        media
            .keyframes
            .retain(|k| !matches!(k.param, KfParam::PosX | KfParam::PosY));
    }
}

const MAX_RECORDING_MS: u64 = 7 * 24 * 60 * 60 * 1000;

fn identity(take: &RecordingTake) -> Result<Option<(String, String, String)>, CutError> {
    // Baked and out-of-window camera artifacts have no editable look. Their
    // existing provenance remains loadable and has no reset target.
    if take.camera_look.is_none() {
        return Ok(None);
    }
    let artifact = take
        .camera_artifact
        .as_ref()
        .ok_or_else(|| bad("camera artifact is missing"))?;
    if take.camera_asset.as_deref().is_none_or(str::is_empty)
        || take.camera_clip.as_deref() == Some("")
        || take.camera_asset == Some(take.asset.clone())
    {
        return Err(bad("camera identity is incomplete"));
    }
    let field = |name| {
        artifact[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("camera artifact identity is invalid"))
    };
    let schema = field("schema")?;
    let capture = field("capture_id")?;
    let id = field("artifact_id")?;
    if schema != "shellx-record/camera-artifact/1"
        || [capture, id].iter().any(|s| {
            s.len() > 128
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        return Err(bad("camera artifact identity is invalid"));
    }
    let video = field("video")?;
    let video_path = Path::new(video);
    if video.contains(['\\', ':'])
        || video_path.is_absolute()
        || video_path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(bad("camera video path is invalid"));
    }
    let hash = artifact["media"]["sha256"]
        .as_str()
        .ok_or_else(|| bad("camera hash is missing"))?;
    if artifact["media"]["duration_ms"]
        .as_u64()
        .is_none_or(|ms| ms == 0 || ms > MAX_RECORDING_MS)
    {
        return Err(bad("camera duration is invalid"));
    }
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad("camera hash is invalid"));
    }
    let mut digest = Sha256::new();
    digest.update(schema.as_bytes());
    digest.update([0]);
    digest.update(capture.as_bytes());
    digest.update([0]);
    digest.update(id.as_bytes());
    let full = format!("{:x}", digest.finalize());
    let track = format!("Camera_{}", &full[..32]);
    // A portable copy places a camera asset in the package media directory.
    // Other captures keep the video relative to their capture folder.
    let path = if video.starts_with("media/sha256/") {
        video.to_owned()
    } else {
        Path::new(&take.capture_folder)
            .join(video)
            .to_string_lossy()
            .into_owned()
    };
    Ok(Some((track, format!("sha256:{hash}"), path)))
}

fn same_path_components(left: &str, right: &str) -> bool {
    let left = left.replace('\\', "/");
    let right = right.replace('\\', "/");
    Path::new(&left)
        .components()
        .eq(Path::new(&right).components())
}

fn valid_transform(t: &ClipTransform, project: &Project, track: &str) -> Result<(), CutError> {
    t.validate_shape()?;
    t.validate_shape_track(project, track)?;
    if !t.scale.is_finite()
        || !(0.0 < t.scale && t.scale <= 1.0)
        || !t.x.is_finite()
        || !(0.0..=1.0).contains(&t.x)
        || !t.y.is_finite()
        || !(0.0..=1.0).contains(&t.y)
        || !t.opacity.is_finite()
        || !(0.0..=1.0).contains(&t.opacity)
    {
        return Err(bad("camera transform is outside edit.transform bounds"));
    }
    Ok(())
}

fn valid_look(
    look: &CameraLook,
    project: &Project,
    track: &str,
    max_ms: u64,
    enforce_bound: bool,
) -> Result<(), CutError> {
    valid_transform(&look.transform, project, track)?;
    if look.keyframes.len() > 4 {
        return Err(bad("camera look has too many channels"));
    }
    let mut seen = 0u8;
    for channel in &look.keyframes {
        if enforce_bound && channel.points.len() > crate::keyframe_limit::MAX_KEYFRAME_POINTS {
            return Err(CutError::new(
                crate::error_codes::INVALID_ARGS,
                format!(
                    "camera look has {} points in {:?}; the limit is {} per channel",
                    channel.points.len(),
                    channel.param,
                    crate::keyframe_limit::MAX_KEYFRAME_POINTS
                ),
                "the separate camera look cannot be rendered safely",
            )
            .with_suggested_action("record again with the camera drawn into the video"));
        }
        let bit = match channel.param {
            KfParam::PosX => 1,
            KfParam::PosY => 2,
            KfParam::Opacity => 4,
            KfParam::Scale => 8,
            KfParam::Volume => return Err(bad("camera look has an audio channel")),
        };
        if seen & bit != 0 || channel.points.is_empty() {
            return Err(bad("camera look has duplicate or empty channels"));
        }
        seen |= bit;
        let mut previous = None;
        for point in &channel.points {
            let valid_value = match channel.param {
                KfParam::PosX | KfParam::PosY | KfParam::Opacity => {
                    (0.0..=1.0).contains(&point.value)
                }
                KfParam::Scale => (0.0..=10.0).contains(&point.value) && point.value > 0.0,
                KfParam::Volume => false,
            };
            if !point.value.is_finite()
                || !valid_value
                || point.t_ms > max_ms
                || previous.is_some_and(|last| point.t_ms <= last)
            {
                return Err(bad("camera look has invalid keyframe points"));
            }
            previous = Some(point.t_ms);
        }
    }
    Ok(())
}

fn valid_slice(media: &crate::types::MediaClip, take: &RecordingTake) -> Result<(), CutError> {
    let duration = take.camera_artifact.as_ref().unwrap()["media"]["duration_ms"]
        .as_u64()
        .unwrap();
    if media.src_in_ms >= media.src_out_ms
        || media.src_out_ms > MAX_RECORDING_MS
        || media.src_out_ms > duration
        || !media.speed.is_finite()
        || !(0.25..=4.0).contains(&media.speed)
        || media.speed_ramp.is_some()
        || media.animation.is_some()
    {
        return Err(bad("camera clip has an invalid source range or speed"));
    }
    Ok(())
}

pub(super) fn validate_take_camera(
    project: &Project,
    take: &RecordingTake,
) -> Result<(), CutError> {
    validate_take_camera_inner(project, take, true)
}

pub(super) fn validate_take_camera_legacy(
    project: &Project,
    take: &RecordingTake,
) -> Result<(), CutError> {
    validate_take_camera_inner(project, take, false)
}

fn validate_take_camera_inner(
    project: &Project,
    take: &RecordingTake,
    enforce_bound: bool,
) -> Result<(), CutError> {
    let Some((track_id, hash, path)) = identity(take)? else {
        return Ok(());
    };
    let asset_id = take.camera_asset.as_ref().unwrap();
    let asset = project
        .assets
        .get(asset_id)
        .ok_or_else(|| bad("camera asset is missing"))?;
    if !same_path_components(&asset.path, &path) || asset.hash != hash {
        return Err(bad("camera asset does not match the recorded artifact"));
    }
    let Some(camera_clip) = take.camera_clip.as_deref() else {
        return valid_look(
            take.camera_look.as_ref().unwrap(),
            project,
            &track_id,
            MAX_RECORDING_MS,
            enforce_bound,
        );
    };
    let (clip_track, index) = project
        .find_clip(camera_clip)
        .ok_or_else(|| bad("camera clip is missing"))?;
    if project.track(clip_track).unwrap().kind != TrackKind::Video {
        return Err(bad("camera clip is not on a video track"));
    }
    let Clip::Media(media) = &project.track(clip_track).unwrap().clips[index] else {
        return Err(bad("camera clip must be video media"));
    };
    if media.asset != *asset_id {
        return Err(bad("camera clip does not play the recorded asset"));
    }
    valid_slice(media, take)?;
    valid_look(
        take.camera_look.as_ref().unwrap(),
        project,
        clip_track,
        MAX_RECORDING_MS,
        enforce_bound,
    )
}

/// Follow the verified camera asset through split, trim and video-track moves.
pub fn camera_look_for_clip(project: &Project, clip: &str) -> Result<CameraLook, CutError> {
    camera_look_for_clip_inner(project, clip, true)
}

pub(super) fn camera_look_for_clip_legacy(
    project: &Project,
    clip: &str,
) -> Result<CameraLook, CutError> {
    camera_look_for_clip_inner(project, clip, false)
}

fn camera_look_for_clip_inner(
    project: &Project,
    clip: &str,
    enforce_bound: bool,
) -> Result<CameraLook, CutError> {
    let (track, index) = project
        .find_clip(clip)
        .ok_or_else(|| bad("camera clip is missing"))?;
    let video_track = project.track(track).unwrap();
    let Clip::Media(media) = &video_track.clips[index] else {
        return Err(bad("camera must be media"));
    };
    if project.recordings.iter().any(|take| {
        take.camera_look.is_none()
            && (take.camera_asset.as_deref() == Some(media.asset.as_str())
                || take.camera_clip.as_deref() == Some(clip))
    }) {
        return Err(CutError::new(
            crate::error_codes::INVALID_ARGS,
            "this recording's camera is part of the video",
            "only a recording with a separate camera look can reset its camera",
        ));
    }
    let mut matches = project.recordings.iter().filter(|take| {
        take.camera_asset.as_deref() == Some(media.asset.as_str()) && take.camera_look.is_some()
    });
    let take = matches
        .next()
        .ok_or_else(|| bad("recorded camera look is missing"))?;
    if matches.next().is_some() {
        return Err(bad("camera asset belongs to multiple recordings"));
    }
    if take.camera_clip.is_none() {
        return Err(bad("recording has no placed camera clip"));
    }
    let (_, hash, path) = identity(take)?.ok_or_else(|| bad("camera identity is missing"))?;
    let asset = project
        .assets
        .get(&media.asset)
        .ok_or_else(|| bad("camera asset is missing"))?;
    if video_track.kind != TrackKind::Video
        || asset.hash != hash
        || !same_path_components(&asset.path, &path)
    {
        return Err(bad("camera clip does not play the recorded camera asset"));
    }
    let original = take
        .camera_look
        .as_ref()
        .ok_or_else(|| bad("recorded camera look is missing"))?;
    valid_look(original, project, track, MAX_RECORDING_MS, enforce_bound)?;
    valid_slice(media, take)?;
    let mut look = original.clone();
    let start = media.src_in_ms;
    let end = media.src_out_ms;
    let duration = video_track.clips[index].timeline_duration_ms();
    for channel in &mut look.keyframes {
        let initial = channel
            .points
            .iter()
            .take_while(|p| p.t_ms <= start)
            .last()
            .or_else(|| channel.points.first())
            .map(|p| p.value)
            .unwrap_or(1.0);
        match channel.param {
            KfParam::PosX => look.transform.x = initial,
            KfParam::PosY => look.transform.y = initial,
            KfParam::Opacity => look.transform.opacity = initial,
            KfParam::Scale => look.transform.scale *= initial,
            KfParam::Volume => unreachable!(),
        }
        let mut points = vec![KfPoint {
            t_ms: 0,
            value: initial,
        }];
        points.extend(
            channel
                .points
                .iter()
                .filter(|p| p.t_ms > start && p.t_ms < end)
                .map(|p| KfPoint {
                    t_ms: ((p.t_ms - start) as f64 / media.speed).round() as u64,
                    value: p.value,
                }),
        );
        if channel.param == KfParam::Scale {
            if initial <= 0.0 {
                return Err(bad("camera scale key is invalid"));
            }
            for point in &mut points {
                point.value /= initial;
            }
        }
        points.dedup_by_key(|point| point.t_ms);
        if points
            .iter()
            .any(|p| p.t_ms > duration || !p.value.is_finite())
        {
            return Err(bad("camera key is outside the clip duration"));
        }
        channel.points = points;
    }
    valid_look(&look, project, track, duration, enforce_bound)?;
    Ok(look)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProjectSettings;
    use serde_json::json;

    fn fixture() -> Project {
        let mut project = Project::new("camera", ProjectSettings::default());
        let artifact = json!({
            "schema": "shellx-record/camera-artifact/1",
            "capture_id": "capture-1",
            "artifact_id": "camera-1",
            "video": "camera.mp4",
            "media": {"sha256": "a".repeat(64), "duration_ms": 1000}
        });
        let take: RecordingTake = serde_json::from_value(json!({
            "id": "take-1", "capture_folder": "/project/cache/screen_record/capture-1",
            "source": "screen.mp4", "events": "events.json",
            "pending_plan": "plan.json", "applied_plan": "plan.json",
            "clip_ids": ["screen"], "asset": "screen-asset", "segments": [],
            "camera_artifact": artifact, "camera_asset": "camera-asset",
            "camera_clip": "camera", "camera_look": {
                "transform": {"x": 0.1, "y": 0.2, "scale": 0.25,
                    "opacity": 1.0, "shape": "circle"},
                "keyframes": [{"param": "scale", "points": [
                    {"t_ms": 0, "value": 1.0},
                    {"t_ms": 500, "value": 0.8}
                ]}]
            }
        }))
        .unwrap();
        let track_id = identity(&take).unwrap().unwrap().0;
        project.assets.insert(
            "screen-asset".into(),
            serde_json::from_value(json!({
                "path": "/project/screen.mp4", "hash": "sha256:screen"
            }))
            .unwrap(),
        );
        let camera_path = Path::new(&take.capture_folder)
            .join("camera.mp4")
            .to_string_lossy()
            .into_owned();
        project.assets.insert(
            "camera-asset".into(),
            serde_json::from_value(json!({
                "path": camera_path,
                "hash": format!("sha256:{}", "a".repeat(64))
            }))
            .unwrap(),
        );
        project.tracks[0].clips.push(
            serde_json::from_value(json!({
                "id": "screen", "asset": "screen-asset",
                "src_in_ms": 0, "src_out_ms": 1000
            }))
            .unwrap(),
        );
        let camera: Clip = serde_json::from_value(json!({
            "id": "camera", "asset": "camera-asset",
            "src_in_ms": 0, "src_out_ms": 1000
        }))
        .unwrap();
        project.tracks.push(crate::types::Track {
            id: track_id,
            kind: TrackKind::Video,
            clips: vec![camera],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        validate_take_camera(&project, &take).unwrap();
        project.recordings.push(take);
        project
    }

    #[test]
    fn camera_reset_rejects_non_camera_asset() {
        let mut project = fixture();
        let ordinary: Clip = serde_json::from_value(json!({
            "id": "ordinary", "asset": "screen-asset",
            "src_in_ms": 0, "src_out_ms": 1000
        }))
        .unwrap();
        project.tracks[0].clips.push(ordinary);
        assert!(super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "ordinary"})
        )
        .is_err());
        project.recordings[0].camera_asset = Some("screen-asset".into());
        assert!(super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "ordinary"})
        )
        .is_err());
    }

    #[test]
    fn camera_reset_after_move_to_another_video_track() {
        let mut project = fixture();
        let base = project.tracks[0].id.clone();
        let target = "camera-overlay-2".to_string();
        let mut overlay = project.tracks[2].clone();
        overlay.id = target.clone();
        overlay.clips.clear();
        project.tracks.push(overlay);
        crate::edit::move_clip(&mut project, "camera", &target, 1000, true).unwrap();
        let (moved_track, _) = project.find_clip("camera").unwrap();
        assert_eq!(moved_track, target);
        super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"}),
        )
        .unwrap();
        let (track, index) = project.find_clip("camera").unwrap();
        let Clip::Media(media) = &project.track(track).unwrap().clips[index] else {
            unreachable!()
        };
        assert_eq!(media.transform.as_ref().unwrap().scale, 0.25);

        crate::edit::move_clip(&mut project, "camera", &base, 1000, true).unwrap();
        let refusal = super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"}),
        )
        .unwrap_err();
        assert_eq!(refusal.message, "shape applies only to overlay video clips");
    }

    #[test]
    fn camera_identity_accepts_both_path_separators() {
        let mut project = fixture();
        let asset = project.assets.get_mut("camera-asset").unwrap();
        asset.path = asset.path.replace('/', "\\");
        validate_take_camera(&project, &project.recordings[0]).unwrap();
        camera_look_for_clip(&project, "camera").unwrap();
    }

    #[test]
    fn camera_reset_rejects_extreme_look_and_invalid_slice() {
        let mut project = fixture();
        project.recordings[0]
            .camera_look
            .as_mut()
            .unwrap()
            .transform
            .scale = f64::MAX;
        assert!(super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"})
        )
        .is_err());
        project.recordings[0]
            .camera_look
            .as_mut()
            .unwrap()
            .transform
            .scale = 0.25;
        project.recordings[0]
            .camera_look
            .as_mut()
            .unwrap()
            .keyframes[0]
            .points[0]
            .value = 10.0;
        assert!(super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"})
        )
        .is_err());
        project.recordings[0]
            .camera_look
            .as_mut()
            .unwrap()
            .keyframes[0]
            .points[0]
            .value = 1.0;
        let camera = match &mut project.tracks[2].clips[0] {
            Clip::Media(media) => media,
            _ => unreachable!(),
        };
        camera.speed = 0.0;
        assert!(super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"})
        )
        .is_err());
    }

    #[test]
    fn camera_take_link_rejects_forged_identity_and_excessive_points() {
        let mut project = fixture();
        let mut take = project.recordings.pop().unwrap();
        take.camera_asset = Some("screen-asset".into());
        assert!(super::super::apply(
            &mut project,
            "screen_record.take_link",
            &json!({"recording": take})
        )
        .is_err());
        take.camera_asset = Some("camera-asset".into());
        take.camera_artifact.as_mut().unwrap()["media"]["duration_ms"] = json!(u64::MAX);
        assert!(super::super::apply(
            &mut project,
            "screen_record.take_link",
            &json!({"recording": take})
        )
        .is_err());
        take.camera_artifact.as_mut().unwrap()["media"]["duration_ms"] = json!(1000);
        take.camera_look.as_mut().unwrap().keyframes[0].points = (0
            ..=crate::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|t_ms| KfPoint {
                t_ms: t_ms as u64,
                value: 1.0,
            })
            .collect();
        let error = super::super::apply(
            &mut project,
            "screen_record.take_link",
            &json!({"recording": take}),
        )
        .unwrap_err();
        assert!(error.message.contains("65 points"), "{error}");
        assert!(error.message.contains("limit is 64"), "{error}");
        assert!(error
            .suggested_action
            .unwrap()
            .contains("camera drawn into the video"));
        super::super::apply_saved(
            &mut project,
            "screen_record.take_link",
            &json!({"recording": take}),
        )
        .unwrap();
        assert_eq!(
            project.recordings[0]
                .camera_look
                .as_ref()
                .unwrap()
                .keyframes[0]
                .points
                .len(),
            65
        );
    }

    #[test]
    fn camera_reset_refuses_an_oversized_saved_look() {
        let mut project = fixture();
        let points: Vec<_> = (0..=crate::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|t_ms| KfPoint {
                t_ms: t_ms as u64,
                value: 1.0,
            })
            .collect();
        project.recordings[0]
            .camera_look
            .as_mut()
            .unwrap()
            .keyframes[0]
            .points = points.clone();
        let args = json!({"clip": "camera"});
        let error =
            super::super::apply(&mut project, "screen_record.camera_reset", &args).unwrap_err();
        assert!(error.message.contains("65 points"), "{error}");
        super::super::apply_saved(&mut project, "screen_record.camera_reset", &args).unwrap();
        let (track, index) = project.find_clip("camera").unwrap();
        let Clip::Media(media) = &project.track(track).unwrap().clips[index] else {
            panic!("expected camera media")
        };
        assert_eq!(media.keyframes[0].points, points);
    }

    #[test]
    fn camera_take_link_keeps_look_when_camera_has_no_timeline_clip() {
        let mut project = fixture();
        let mut take = project.recordings.pop().unwrap();
        take.camera_clip = None;
        project.tracks.pop();
        super::super::apply(
            &mut project,
            "screen_record.take_link",
            &json!({"recording": take}),
        )
        .unwrap();
        assert!(project.recordings[0].camera_look.is_some());
    }

    #[test]
    fn legacy_recording_without_camera_fields_roundtrips_byte_identical() {
        let mut project = fixture();
        let mut old = serde_json::to_value(project.recordings.remove(0)).unwrap();
        for field in [
            "camera_artifact",
            "camera_clip",
            "camera_asset",
            "camera_look",
        ] {
            old.as_object_mut().unwrap().remove(field);
        }
        project
            .recordings
            .push(serde_json::from_value(old).unwrap());
        let bytes = serde_json::to_vec(&project).unwrap();
        let loaded: Project = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&loaded).unwrap());
        assert!(!String::from_utf8(bytes).unwrap().contains("camera_look"));
    }

    #[test]
    fn valid_camera_reset_keeps_recorded_look() {
        let mut project = fixture();
        super::super::apply(
            &mut project,
            "screen_record.camera_reset",
            &json!({"clip": "camera"}),
        )
        .unwrap();
        let Clip::Media(media) = &project.tracks[2].clips[0] else {
            unreachable!()
        };
        assert_eq!(media.transform.as_ref().unwrap().scale, 0.25);
        assert_eq!(media.keyframes[0].points[1].value, 0.8);
    }
}
