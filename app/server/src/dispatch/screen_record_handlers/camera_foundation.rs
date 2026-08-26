//! CameraArtifact@1 placement for the recorder polish orchestrator.
//!
//! This consumes an already-validated capture artifact. It does not enumerate,
//! open, preview, or record a native camera, so it must not be treated as B7
//! live-camera readiness.

use super::*;
use serde_json::json;
use std::path::Path;

#[derive(Default)]
pub(super) struct CameraPlacement {
    pub(super) asset_id: Option<String>,
    pub(super) clip_id: Option<String>,
    pub(super) track_id: Option<String>,
}

pub(super) async fn place_camera_artifact(
    state: &AppState,
    actor: &Actor,
    camera: Option<(&Path, &record_core::CameraArtifact)>,
) -> Result<CameraPlacement, CutError> {
    let Some((video_path, artifact)) = camera else {
        return Ok(CameraPlacement::default());
    };

    // A synchronized camera capture is an independently editable timeline
    // asset, not merely a baked EditPlan webcam bubble. The first-frame offset
    // is the immutable shared CaptureClock placement; the explicit source range
    // guarantees no last camera frame is extrapolated past its actual stream.
    let imported = Box::pin(dispatch(
        state,
        "media.import",
        json!({
            "path": video_path.display().to_string(),
            "proxy": false,
            "rationale": "auto: screen_record.polish synchronized camera artifact",
        }),
        actor.clone(),
    ))
    .await;
    if let Some(error) = screen_record_polish_subverb_error("camera artifact import", &imported) {
        return Err(error);
    }
    let asset_id = imported
        .result
        .as_ref()
        .and_then(|result| result["asset_id"].as_str())
        .map(String::from)
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "camera artifact import returned no asset id",
                "media.import succeeded but did not return result.asset_id",
            )
        })?;

    let track_id = "v_camera".to_string();
    let exists = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .map(|store| {
                store
                    .project
                    .tracks
                    .iter()
                    .any(|candidate| candidate.id == track_id)
            })
            .unwrap_or(false)
    };
    if !exists {
        let added = Box::pin(dispatch(
            state,
            "edit.add_track",
            json!({
                "kind": "video",
                "id": track_id,
                "rationale": "auto: screen_record.polish synchronized camera track",
            }),
            actor.clone(),
        ))
        .await;
        if let Some(error) = screen_record_polish_subverb_error("camera track creation", &added) {
            return Err(error);
        }
        // Existing polish still owns the legacy bubble render. Keep this raw
        // artifact editable but hidden until B7 supplies its visible workflow.
        let hidden = Box::pin(dispatch(
            state,
            "edit.track_visible",
            json!({
                "track": track_id,
                "on": false,
                "rationale": "auto: screen_record.polish camera artifact compatibility track",
            }),
            actor.clone(),
        ))
        .await;
        if let Some(error) = screen_record_polish_subverb_error("camera track visibility", &hidden)
        {
            return Err(error);
        }
    }

    let inserted = Box::pin(dispatch(
        state,
        "edit.insert",
        json!({
            "asset": asset_id,
            "track": track_id,
            "at_ms": artifact.clock.first_frame_offset_ms,
            "src_range_ms": [0, artifact.media.duration_ms],
            "ripple": false,
            "rationale": "auto: screen_record.polish synchronized camera placement",
        }),
        actor.clone(),
    ))
    .await;
    if let Some(error) = screen_record_polish_subverb_error("camera artifact insert", &inserted) {
        return Err(error);
    }
    let clip_id = inserted
        .result
        .as_ref()
        .and_then(|result| result["clip_id"].as_str())
        .map(String::from);

    Ok(CameraPlacement {
        asset_id: Some(asset_id),
        clip_id,
        track_id: Some(track_id),
    })
}
