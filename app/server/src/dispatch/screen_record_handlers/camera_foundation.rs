//! CameraArtifact@1 placement for the recorder polish orchestrator.
//!
//! This consumes an already-validated capture artifact. It does not enumerate,
//! open, preview, or record a native camera, so it must not be treated as B7
//! live-camera readiness. The server owns the durable import one artifact at a
//! time: an identity-bound track lets a retry after a restart adopt a completed
//! import/track/clip rather than create a second camera take.

use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(super) struct CameraPlacement {
    pub(super) asset_id: Option<String>,
    pub(super) clip_id: Option<String>,
    pub(super) track_id: Option<String>,
}

#[derive(Debug)]
enum CameraOwner {
    /// A prior attempt persisted the full camera placement. It is safe to
    /// return it unchanged: the track is deliberately editable after import.
    Existing(CameraPlacement),
    /// No durable clip owns this artifact yet. A previous interrupted attempt
    /// may nevertheless have committed the media.import op, in which case the
    /// next attempt must reuse that asset instead of importing a duplicate.
    Pending {
        project_dir: PathBuf,
        reusable_asset_id: Option<String>,
    },
}

pub(super) async fn place_camera_artifact(
    state: &AppState,
    actor: &Actor,
    camera: Option<(&Path, &record_core::CameraArtifact)>,
) -> Result<CameraPlacement, CutError> {
    let Some((video_path, artifact)) = camera else {
        return Ok(CameraPlacement::default());
    };

    let (video_path, byte_length) = verify_camera_source(video_path, artifact)?;
    let track_id = camera_track_id(artifact);
    match durable_camera_owner(state, &track_id, &video_path, artifact).await? {
        CameraOwner::Existing(placement) => Ok(placement),
        CameraOwner::Pending {
            project_dir,
            reusable_asset_id,
        } => {
            let asset_id = if let Some(asset_id) = reusable_asset_id {
                asset_id
            } else {
                import_camera_artifact(
                    state,
                    actor,
                    artifact,
                    &video_path,
                    byte_length,
                    &project_dir,
                )
                .await?
            };

            ensure_camera_track(state, actor, &track_id).await?;
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
            if let Some(error) =
                screen_record_polish_subverb_error("camera artifact insert", &inserted)
            {
                return Err(error);
            }
            let clip_id = inserted
                .result
                .as_ref()
                .and_then(|result| result["clip_id"].as_str())
                .map(String::from)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::JOB_FAILED,
                        "camera artifact insert returned no clip id",
                        "edit.insert succeeded but did not return result.clip_id",
                    )
                })?;

            Ok(CameraPlacement {
                asset_id: Some(asset_id),
                clip_id: Some(clip_id),
                track_id: Some(track_id),
            })
        }
    }
}

/// One stable, collision-resistant identity for the exact `(capture, artifact)`
/// pair. `artifact_id` is only unique within a capture, so using it alone would
/// let unrelated recordings silently share a track after a restart.
fn camera_track_id(artifact: &record_core::CameraArtifact) -> String {
    let mut hasher = Sha256::new();
    hasher.update(artifact.schema.as_bytes());
    hasher.update([0]);
    hasher.update(artifact.capture_id.as_bytes());
    hasher.update([0]);
    hasher.update(artifact.artifact_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    // 128 bits keeps an accidental collision beyond the useful lifetime of a
    // project while keeping the user-visible track id compact and legal.
    format!("v_camera_{}", &digest[..32])
}

/// Recheck the sealed camera source at the server boundary. The initial
/// capture-artifact resolver has already done this; repeating it here keeps an
/// import retry from accepting a replacement between discovery and ownership.
fn verify_camera_source(
    video_path: &Path,
    artifact: &record_core::CameraArtifact,
) -> Result<(PathBuf, u64), CutError> {
    artifact.validate().map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "camera artifact is invalid before durable import",
            error.to_string(),
        )
    })?;
    let byte_length = std::fs::metadata(video_path)
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                format!(
                    "could not inspect camera artifact before durable import at {}",
                    video_path.display()
                ),
                error.to_string(),
            )
        })?
        .len();
    let (canonical, _) = crate::dispatch::media::verify_attested_media_source(
        video_path,
        &artifact.media.sha256,
        byte_length,
    )?;
    Ok((canonical, byte_length))
}

async fn durable_camera_owner(
    state: &AppState,
    track_id: &str,
    video_path: &Path,
    artifact: &record_core::CameraArtifact,
) -> Result<CameraOwner, CutError> {
    let expected_path = video_path.display().to_string();
    let expected_hash = camera_asset_hash(artifact);
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let project_dir = store.dir.clone();
    let same_path_assets = store
        .project
        .assets
        .iter()
        .filter(|(_, asset)| asset.path == expected_path)
        .collect::<Vec<_>>();
    if same_path_assets
        .iter()
        .any(|(_, asset)| asset.hash != expected_hash)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "camera artifact source identity conflicts with an imported asset",
            "an existing asset has the sealed camera path but not its exact SHA-256 identity",
        ));
    }
    let reusable_asset_ids = same_path_assets
        .iter()
        .map(|(asset_id, _)| (*asset_id).clone())
        .collect::<Vec<_>>();

    let Some(track) = store
        .project
        .tracks
        .iter()
        .find(|candidate| candidate.id == track_id)
    else {
        return unique_pending_camera_owner(project_dir, reusable_asset_ids);
    };
    if track.kind != cut_core::TrackKind::Video {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "camera artifact track has the wrong kind",
            format!("durable camera track '{track_id}' is not a video track"),
        ));
    }

    let matching = track
        .clips
        .iter()
        .filter_map(|clip| match clip {
            cut_core::Clip::Media(media)
                if store.project.assets.get(&media.asset).is_some_and(|asset| {
                    asset.path == expected_path && asset.hash == expected_hash
                }) =>
            {
                Some((media.id.clone(), media.asset.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [(clip_id, asset_id)] => Ok(CameraOwner::Existing(CameraPlacement {
            asset_id: Some(asset_id.clone()),
            clip_id: Some(clip_id.clone()),
            track_id: Some(track_id.to_string()),
        })),
        [] if track.clips.is_empty() => {
            unique_pending_camera_owner(project_dir, reusable_asset_ids)
        }
        [] => Err(CutError::new(
            error_codes::CONFLICT,
            "camera artifact track is occupied by different media",
            format!("durable camera track '{track_id}' has no clip for the sealed camera source"),
        )),
        _ => Err(CutError::new(
            error_codes::CONFLICT,
            "camera artifact track has duplicate camera clips",
            format!(
                "durable camera track '{track_id}' has multiple clips for one sealed camera source"
            ),
        )),
    }
}

fn camera_asset_hash(artifact: &record_core::CameraArtifact) -> String {
    format!("sha256:{}", artifact.media.sha256)
}

fn unique_pending_camera_owner(
    project_dir: PathBuf,
    reusable_asset_ids: Vec<String>,
) -> Result<CameraOwner, CutError> {
    let reusable_asset_id = match reusable_asset_ids.as_slice() {
        [] => None,
        [asset_id] => Some(asset_id.clone()),
        _ => {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "camera artifact has multiple unowned imported assets",
                "cannot choose a durable camera owner after an interrupted import; resolve the duplicate assets before retrying",
            ));
        }
    };
    Ok(CameraOwner::Pending {
        project_dir,
        reusable_asset_id,
    })
}

async fn import_camera_artifact(
    state: &AppState,
    actor: &Actor,
    artifact: &record_core::CameraArtifact,
    video_path: &Path,
    byte_length: u64,
    project_dir: &Path,
) -> Result<String, CutError> {
    // `expected_sha256` / `expected_byte_length` are deliberately internal
    // media.import guards. Calling the trusted handler directly is required:
    // public dispatch rejects these internal fields, while this server owner
    // has just verified the sealed CameraArtifact@1 source.
    let imported = crate::dispatch::media::media_import(
        state,
        json!({
            "path": video_path.display().to_string(),
            "proxy": false,
            "rationale": "auto: screen_record.polish synchronized camera artifact",
            "expected_project_dir": project_dir.display().to_string(),
            "expected_sha256": artifact.media.sha256,
            "expected_byte_length": byte_length,
        }),
        actor.clone(),
    )
    .await?;
    if let Some(error) = screen_record_polish_subverb_error("camera artifact import", &imported) {
        return Err(error);
    }
    imported
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
        })
}

async fn ensure_camera_track(
    state: &AppState,
    actor: &Actor,
    track_id: &str,
) -> Result<(), CutError> {
    let visible = {
        let guard = state.project.read().await;
        guard.as_ref().and_then(|store| {
            store
                .project
                .tracks
                .iter()
                .find(|candidate| candidate.id == track_id)
                .map(|track| track.visible)
        })
    };
    if visible.is_none() {
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
    }
    if visible == Some(false) {
        return Ok(());
    }

    // Existing polish still owns the legacy bubble render. Keep this raw
    // artifact editable but hidden until B7 supplies its visible workflow. An
    // interrupted first attempt may have persisted add_track but not this
    // visibility op, so an empty recovered owner needs the same correction.
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
    if let Some(error) = screen_record_polish_subverb_error("camera track visibility", &hidden) {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::{
        CameraClockRange, CameraMediaFacts, CameraTerminalState, CAMERA_ARTIFACT_SCHEMA,
    };
    use sha2::{Digest, Sha256};

    fn artifact(capture_id: &str, artifact_id: &str) -> record_core::CameraArtifact {
        record_core::CameraArtifact::new(
            capture_id,
            artifact_id,
            "camera/camera.mp4",
            CameraClockRange {
                first_frame_offset_ms: 100,
                end_frame_offset_ms: 1_100,
            },
            CameraMediaFacts {
                width: 640,
                height: 480,
                fps_num: 30,
                fps_den: 1,
                frame_count: 30,
                duration_ms: 1_000,
                sha256: "a".repeat(64),
            },
            CameraTerminalState::Complete,
        )
        .unwrap()
    }

    #[test]
    fn camera_track_identity_binds_both_capture_and_artifact() {
        let first = artifact("capture_a", "camera_01");
        assert_eq!(first.schema, CAMERA_ARTIFACT_SCHEMA);
        assert_eq!(camera_track_id(&first), camera_track_id(&first));
        assert_ne!(
            camera_track_id(&first),
            camera_track_id(&artifact("capture_b", "camera_01")),
            "artifact ids are only unique within one capture"
        );
        assert_ne!(
            camera_track_id(&first),
            camera_track_id(&artifact("capture_a", "camera_02"))
        );
    }

    async fn wrong_hash_owner_fixture(
        completed_track: bool,
    ) -> (
        tempfile::TempDir,
        AppState,
        record_core::CameraArtifact,
        PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("camera-owner.cutproj");
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": "camera-owner", "dir": project_dir}),
            Actor::system(),
        )
        .await;
        assert!(
            created.ok,
            "camera test project creation failed: {:?}",
            created.error
        );

        let video_path = temp.path().join("camera.mp4");
        let video_bytes = b"sealed camera fixture";
        std::fs::write(&video_path, video_bytes).unwrap();
        let artifact = record_core::CameraArtifact::new(
            "capture_01",
            "camera_01",
            "camera/camera.mp4",
            CameraClockRange {
                first_frame_offset_ms: 100,
                end_frame_offset_ms: 1_100,
            },
            CameraMediaFacts {
                width: 640,
                height: 480,
                fps_num: 30,
                fps_den: 1,
                frame_count: 30,
                duration_ms: 1_000,
                sha256: format!("{:x}", Sha256::digest(video_bytes)),
            },
            CameraTerminalState::Complete,
        )
        .unwrap();
        let canonical_video = video_path.canonicalize().unwrap();
        let wrong_asset_id = "a_camera_wrong".to_string();
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some(wrong_asset_id.clone()),
                    cut_core::Asset {
                        path: canonical_video.display().to_string(),
                        hash: format!("sha256:{}", "b".repeat(64)),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    Actor::system(),
                    None,
                )
                .unwrap();
        }
        if completed_track {
            let track_id = camera_track_id(&artifact);
            let added = dispatch(
                &state,
                "edit.add_track",
                json!({"kind": "video", "id": track_id}),
                Actor::system(),
            )
            .await;
            assert!(added.ok, "camera fixture track failed: {:?}", added.error);
            let inserted = dispatch(
                &state,
                "edit.insert",
                json!({
                    "asset": wrong_asset_id,
                    "track": track_id,
                    "at_ms": 100,
                    "src_range_ms": [0, 1000],
                    "ripple": false,
                }),
                Actor::system(),
            )
            .await;
            assert!(
                inserted.ok,
                "camera fixture clip insertion failed: {:?}",
                inserted.error
            );
        }
        (temp, state, artifact, canonical_video)
    }

    #[tokio::test]
    async fn camera_owner_rejects_same_path_assets_with_a_wrong_hash() {
        for completed_track in [false, true] {
            let (_temp, state, artifact, video_path) =
                wrong_hash_owner_fixture(completed_track).await;
            let error =
                durable_camera_owner(&state, &camera_track_id(&artifact), &video_path, &artifact)
                    .await
                    .expect_err(
                        "same-path asset with the wrong SHA-256 must not be reused or adopted",
                    );
            assert_eq!(error.code, error_codes::CONFLICT);
            assert_eq!(
                error.message,
                "camera artifact source identity conflicts with an imported asset"
            );
        }
    }
}
