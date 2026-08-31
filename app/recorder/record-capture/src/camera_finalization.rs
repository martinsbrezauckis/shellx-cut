//! Private close → probe → hash → anchored-publish camera finalization.

use std::fs::File;

use record_core::{CameraMediaFacts, Result};

use crate::camera_finalization_anchored::{
    enforce_non_writable_mode, ensure_non_writable_mode, AnchoredDirectory,
};
use crate::camera_finalization_error::finalization_error;
#[path = "camera_finalization_hooks.rs"]
mod hooks;
use crate::camera_finalization_identity::{
    hash_verified_file, verify_nameless_stage, VerifiedRegularFile,
};
use crate::camera_finalization_owner::CameraCaptureDirectory;
#[path = "camera_finalization_parent.rs"]
mod parent;
use crate::camera_finalization_paths::resolve_capture_owned_paths;
use crate::camera_finalization_publication::{link_no_replace, verify_published};
use hooks::{production_hooks, FinalizationHooks};
use parent::revalidate_published_parent;

#[path = "camera_finalization_seal.rs"]
mod verified_seal;

pub(crate) use verified_seal::CameraMediaSeal;

// Windows cannot inherit Linux's nameless-O_TMPFILE proof. Its companion keeps
// the capture root, leaf parent, and created leaf open by handle, then gives
// Capture Engine a byte stream over that exact leaf. Keep all such authority
// under this finalization module so adapters cannot mint a `CameraMediaSeal`.
#[cfg(all(windows, feature = "capture-windows"))]
#[path = "camera_finalization_windows.rs"]
mod windows_no_replace;
#[cfg(all(windows, feature = "capture-windows"))]
#[path = "camera_finalization_windows_stage.rs"]
mod windows_stage;
#[cfg(all(windows, feature = "capture-windows"))]
#[path = "camera_finalization_windows_stream.rs"]
mod windows_stream;
#[cfg(all(windows, feature = "capture-windows"))]
pub(crate) use windows_no_replace::finalize_windows_no_replace;
#[cfg(all(windows, feature = "capture-windows"))]
pub(crate) use windows_stage::{reserve_windows_no_replace_output, WindowsNoReplaceCameraStage};

#[cfg(all(test, target_os = "linux"))]
#[path = "camera_finalization_test_seam.rs"]
mod test_seam;
#[cfg(all(test, target_os = "linux"))]
pub(super) use test_seam::{
    finalize_after_close_for_test, finalize_after_close_with_post_sync_for_test,
    finalize_after_close_with_sync_for_test,
};

/// FPS may differ by at most one thousandth of a frame per second. The native
/// probe rounds FPS at this one explicit boundary; duration is already in the
/// shared millisecond clock and therefore must match exactly.
const MAX_FPS_DRIFT_MILLI: u128 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredCameraMediaFacts {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps_num: u32,
    pub(crate) fps_den: u32,
    pub(crate) frame_count: u64,
    pub(crate) duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeasuredCameraMediaFacts {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps_num: u32,
    pub(crate) fps_den: u32,
    pub(crate) frame_count: u64,
    pub(crate) duration_ms: u64,
}

/// The adapter may declare only a logical destination. It cannot provide a
/// staged host path: a closer returns one closed, readable, nameless stage on
/// the capture filesystem instead.
#[derive(Debug, Clone)]
pub(crate) struct StagedCameraMedia {
    pub(crate) artifact_id: String,
    pub(crate) video: String,
    pub(crate) declared: DeclaredCameraMediaFacts,
}

/// # Safety
///
/// `close_media` may return only after the native writer has flushed, closed,
/// and retired every writable descriptor or alias for this media. The returned
/// descriptor must be read-only and nameless (`O_TMPFILE` on Linux). Without
/// fs-verity, this finalizer cannot prove that an equal-privilege process will
/// not reacquire write authority. Implement this only for a trusted native
/// closer and capture-directory owner that retain exclusive authority from
/// close through the sealing hand-off; other filesystem authority models fail
/// closed because the descriptor and published mode checks cannot prove them.
/// The repeated post-sync checks close races while that authority is retained;
/// they do not promise post-handoff protection or absolute no-TOCTOU without
/// fs-verity and exclusive directory authority.
pub(crate) unsafe trait CameraNativeCloser {
    fn close_media(&mut self) -> Result<File>;
}

pub(crate) trait CameraMediaProbe {
    /// Measure the supplied closed regular-file handle, not a caller path.
    fn measure(&self, closed_file: &File) -> Result<MeasuredCameraMediaFacts>;
}

pub(crate) fn finalize_staged_camera_media(
    closer: &mut impl CameraNativeCloser,
    probe: &impl CameraMediaProbe,
    capture: &CameraCaptureDirectory,
    staged: &StagedCameraMedia,
) -> Result<CameraMediaSeal> {
    let closed_stage = closer.close_media()?;
    finalize_closed(probe, capture, staged, closed_stage, production_hooks())
}

fn finalize_closed<
    AfterPathsAnchored,
    AfterStagedHash,
    AfterLink,
    AfterPublicationVerify,
    SyncFile,
    AfterFileSync,
    SyncDirectory,
    AfterDirectorySync,
>(
    probe: &impl CameraMediaProbe,
    capture: &CameraCaptureDirectory,
    staged: &StagedCameraMedia,
    closed_stage: File,
    hooks: FinalizationHooks<
        AfterPathsAnchored,
        AfterStagedHash,
        AfterLink,
        AfterPublicationVerify,
        SyncFile,
        AfterFileSync,
        SyncDirectory,
        AfterDirectorySync,
    >,
) -> Result<CameraMediaSeal>
where
    AfterPathsAnchored: FnOnce() -> Result<()>,
    AfterStagedHash: FnOnce() -> Result<()>,
    AfterLink: FnOnce() -> Result<()>,
    AfterPublicationVerify: FnOnce() -> Result<()>,
    SyncFile: FnOnce(&File) -> Result<()>,
    AfterFileSync: FnOnce() -> Result<()>,
    SyncDirectory: FnOnce(&AnchoredDirectory) -> Result<()>,
    AfterDirectorySync: FnOnce() -> Result<()>,
{
    let FinalizationHooks {
        after_paths_anchored,
        after_staged_hash,
        after_link,
        after_publication_verify,
        sync_file,
        after_file_sync,
        sync_directory,
        after_directory_sync,
    } = hooks;
    let paths = resolve_capture_owned_paths(capture, &staged.video)?;
    after_paths_anchored()?;
    paths.revalidate_owner()?;
    let stage = verify_nameless_stage(closed_stage, paths.capture_filesystem())?;
    enforce_non_writable_mode(stage.file())?;
    let measured = probe.measure(stage.file())?;
    verify_declared_facts(&staged.declared, &measured)?;
    let (sha256, bytes) = hash_verified_file(&stage)?;
    after_staged_hash()?;
    let (stable_sha256, stable_bytes) = hash_verified_file(&stage)?;
    if (sha256, bytes) != (stable_sha256.clone(), stable_bytes) {
        return Err(finalization_error(
            "camera staged media changed after hashing",
            "the nameless stage must remain byte-identical until anchored publication",
        ));
    }
    let media = measured_media(&measured, stable_sha256.clone());
    CameraMediaSeal::validate_finalizer_inputs(
        &staged.artifact_id,
        &staged.video,
        &media,
        stable_bytes,
    )?;

    link_no_replace(&paths, &stage)?;
    after_link()?;
    let published = verify_published(&paths, &stage)?;
    enforce_non_writable_mode(published.file().file())?;
    verify_published_media(
        probe,
        published.file(),
        &measured,
        &stable_sha256,
        stable_bytes,
    )?;
    after_publication_verify()?;
    drop(published);
    let final_publication = verify_published(&paths, &stage)?;
    verify_published_media(
        probe,
        final_publication.file(),
        &measured,
        &stable_sha256,
        stable_bytes,
    )?;
    let durable_publication = verify_published(&paths, &stage)?;
    verify_published_media(
        probe,
        durable_publication.file(),
        &measured,
        &stable_sha256,
        stable_bytes,
    )?;
    revalidate_published_parent(&paths, &durable_publication)?;
    sync_file(durable_publication.file().file())?;
    after_file_sync()?;
    let post_file_sync = verify_published(&paths, &stage)?;
    verify_published_media(
        probe,
        post_file_sync.file(),
        &measured,
        &stable_sha256,
        stable_bytes,
    )?;
    revalidate_published_parent(&paths, &post_file_sync)?;
    sync_directory(post_file_sync.destination_parent())?;
    after_directory_sync()?;
    let post_directory_sync = verify_published(&paths, &stage)?;
    verify_published_media(
        probe,
        post_directory_sync.file(),
        &measured,
        &stable_sha256,
        stable_bytes,
    )?;
    revalidate_published_parent(&paths, &post_directory_sync)?;
    Ok(CameraMediaSeal::from_finalizer(
        staged.artifact_id.clone(),
        staged.video.clone(),
        media,
        stable_bytes,
    ))
}

fn verify_published_media(
    probe: &impl CameraMediaProbe,
    published: &VerifiedRegularFile,
    expected: &MeasuredCameraMediaFacts,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<()> {
    ensure_non_writable_mode(published.file())?;
    let measured = probe.measure(published.file())?;
    if &measured != expected {
        return Err(finalization_error(
            "camera published media facts changed",
            "the anchored published descriptor no longer measures as the sealed media",
        ));
    }
    let (sha256, bytes) = hash_verified_file(published)?;
    ensure_non_writable_mode(published.file())?;
    if sha256 != expected_sha256 || bytes != expected_bytes {
        return Err(finalization_error(
            "camera published media bytes changed",
            "the anchored published descriptor no longer hashes as the sealed media",
        ));
    }
    Ok(())
}

fn measured_media(measured: &MeasuredCameraMediaFacts, sha256: String) -> CameraMediaFacts {
    CameraMediaFacts {
        width: measured.width,
        height: measured.height,
        fps_num: measured.fps_num,
        fps_den: measured.fps_den,
        frame_count: measured.frame_count,
        duration_ms: measured.duration_ms,
        sha256,
    }
}

fn verify_declared_facts(
    declared: &DeclaredCameraMediaFacts,
    measured: &MeasuredCameraMediaFacts,
) -> Result<()> {
    if measured.width == 0
        || measured.height == 0
        || measured.fps_num == 0
        || measured.fps_den == 0
        || measured.frame_count == 0
        || measured.duration_ms == 0
    {
        return Err(finalization_error(
            "camera media probe returned incomplete facts",
            "final camera media must have non-zero dimensions, FPS, frame count, and duration",
        ));
    }
    if (declared.width, declared.height) != (measured.width, measured.height) {
        return Err(finalization_error(
            "camera media dimensions do not match the declared capture",
            "dimensions are exact final-media facts",
        ));
    }
    if !fps_within_declared_bound(declared, measured) {
        return Err(finalization_error(
            "camera media FPS does not match the declared capture",
            "FPS differs by more than the one-thousandth FPS finalization bound",
        ));
    }
    if declared.frame_count != measured.frame_count {
        return Err(finalization_error(
            "camera media frame count does not match the declared capture",
            "decoded frame count is an exact finalization fact",
        ));
    }
    if declared.duration_ms != measured.duration_ms {
        return Err(finalization_error(
            "camera media duration does not match the declared capture",
            "duration is already rounded to the shared CaptureClock millisecond boundary",
        ));
    }
    Ok(())
}

fn fps_within_declared_bound(
    declared: &DeclaredCameraMediaFacts,
    measured: &MeasuredCameraMediaFacts,
) -> bool {
    if declared.fps_num == 0 || declared.fps_den == 0 {
        return false;
    }
    let difference = (u128::from(declared.fps_num) * u128::from(measured.fps_den))
        .abs_diff(u128::from(measured.fps_num) * u128::from(declared.fps_den));
    difference * 1_000
        <= MAX_FPS_DRIFT_MILLI * u128::from(declared.fps_den) * u128::from(measured.fps_den)
}
