//! Sealed private-camera asset, hidden track, and clip materialization.

use std::path::{Component, Path};

use cut_core::{
    error_codes, Actor, Asset, AtomicMediaInsert, AtomicMediaInsertTrack, CutError,
    MutationRequest, ProjectStore, TrackKind,
};
use record_core::CameraArtifact;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{binding::current_revision, PrivateWindowsCameraProjectOwner, OWNER_SCHEMA};

/// The only private projection. It intentionally excludes device identity and
/// native path details; consumers may publish the listed operation records by
/// their normal server event path when this owner is later admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CameraPlacement {
    pub(super) artifact: CameraArtifact,
    pub(super) asset_id: String,
    pub(super) clip_id: String,
    pub(super) track_id: String,
    pub(super) placement_op_id: String,
    pub(super) already_applied: bool,
}

pub(super) fn place_sealed(
    owner: &PrivateWindowsCameraProjectOwner,
    store: &mut ProjectStore,
    artifact: CameraArtifact,
) -> Result<CameraPlacement, CutError> {
    if artifact.capture_id != owner.capture_id {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "sealed private camera artifact belongs to another capture",
            "place only the artifact retained by this exact screen reservation",
        ));
    }
    let video = super::super::plain_existing_file_under_dir(
        &owner.capture_dir,
        &owner.capture_dir.join(&artifact.video),
        "sealed private camera artifact",
        "discard the incomplete camera artifact and retry a new recording",
    )?;
    let bytes = std::fs::metadata(&video)
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "could not inspect sealed private camera artifact",
                error.to_string(),
            )
        })?
        .len();
    let (video, _) =
        crate::dispatch::verify_attested_media_source(&video, &artifact.media.sha256, bytes)?;
    let asset_path = project_relative_asset_path(&owner.project_dir, &video)?;
    let track_id = camera_track_id(&owner.project_identity, &artifact);
    let placement_revision = current_revision(store)?;
    let key = placement_key(&owner.project_identity, &artifact, &track_id);
    let actor = Actor::system().with_request(MutationRequest {
        caller: OWNER_SCHEMA.into(),
        request_id: format!("camera-{}-{}", artifact.capture_id, artifact.artifact_id),
        fingerprint: format!("sha256:{key}"),
        expected_revision: Some(placement_revision.clone()),
    });
    let transaction = AtomicMediaInsert {
        idempotency_key: key,
        asset: Asset {
            // `Asset.path` supports project-relative paths. The captured source
            // stays inside the project capture root, so project state never
            // publishes an absolute host path.
            path: asset_path.clone(),
            hash: format!("sha256:{}", artifact.media.sha256),
            probe: Some(json!({
                "kind": "video",
                "width": artifact.media.width,
                "height": artifact.media.height,
                "duration_ms": artifact.media.duration_ms,
                "fps_num": artifact.media.fps_num,
                "fps_den": artifact.media.fps_den,
                "frame_count": artifact.media.frame_count,
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
        insert: json!({
            "track": track_id,
            "at_ms": artifact.clock.first_frame_offset_ms,
            "src_range_ms": [0, artifact.media.duration_ms],
            "ripple": false,
        }),
        binding: json!({
            "schema": OWNER_SCHEMA,
            "project_identity": owner.project_identity,
            "accepted_revision": owner.accepted_revision,
            "placement_revision": placement_revision,
            "capture_id": owner.capture_id,
            "artifact_id": artifact.artifact_id,
            "track_id": track_id,
            "asset_path": asset_path,
            "sealed": {
                "sha256": artifact.media.sha256,
                "bytes": bytes,
                "first_frame_offset_ms": artifact.clock.first_frame_offset_ms,
                "end_frame_offset_ms": artifact.clock.end_frame_offset_ms,
                "terminal_state": artifact.terminal_state,
            },
        }),
        // Track creation/hiding, asset registration, and edit.insert all stage
        // against one cloned project and commit as one replayable Undo receipt.
        // A collision or insert failure therefore leaves no empty camera track.
        track: Some(AtomicMediaInsertTrack {
            id: track_id.clone(),
            kind: TrackKind::Video,
            visible: false,
        }),
    };
    let result = store.apply_atomic_media_insert(
        transaction,
        actor,
        Some("auto: private Windows camera sealed placement".into()),
    )?;
    Ok(CameraPlacement {
        artifact,
        asset_id: result.asset_id,
        clip_id: result.clip_id,
        track_id,
        placement_op_id: result.op.op_id,
        already_applied: result.already_applied,
    })
}

/// Convert the fenced, sealed capture source into the project-relative Asset
/// representation used by project loading, cache rebuild, packaging, and
/// media relink. No public project state needs the machine-specific path.
fn project_relative_asset_path(project_dir: &Path, video: &Path) -> Result<String, CutError> {
    let project_dir = std::fs::canonicalize(project_dir).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not resolve private camera project-relative asset path",
            error.to_string(),
        )
    })?;
    let relative = video.strip_prefix(&project_dir).map_err(|_| {
        CutError::new(
            error_codes::CONFLICT,
            "sealed private camera artifact is outside the open project",
            "camera assets must remain under the exact project capture root",
        )
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "sealed private camera artifact has an unsafe project-relative path",
            "camera assets must use a plain non-empty project-relative path",
        ));
    }
    relative
        .to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "sealed private camera artifact path is not valid UTF-8",
                "camera assets require a project-relative UTF-8 path",
            )
        })
}

fn camera_track_id(project_identity: &str, artifact: &CameraArtifact) -> String {
    let mut digest = Sha256::new();
    digest.update(OWNER_SCHEMA.as_bytes());
    digest.update(project_identity.as_bytes());
    digest.update(artifact.capture_id.as_bytes());
    digest.update(artifact.artifact_id.as_bytes());
    format!("v_camera_{}", &format!("{:x}", digest.finalize())[..32])
}

fn placement_key(project_identity: &str, artifact: &CameraArtifact, track_id: &str) -> String {
    let mut digest = Sha256::new();
    for part in [
        OWNER_SCHEMA,
        project_identity,
        artifact.capture_id.as_str(),
        artifact.artifact_id.as_str(),
        artifact.media.sha256.as_str(),
        track_id,
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    digest.update(artifact.media.duration_ms.to_be_bytes());
    digest.update(artifact.clock.first_frame_offset_ms.to_be_bytes());
    digest.update(artifact.clock.end_frame_offset_ms.to_be_bytes());
    format!("{:x}", digest.finalize())
}
