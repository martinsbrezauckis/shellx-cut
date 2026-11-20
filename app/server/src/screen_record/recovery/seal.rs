//! Seal a capture whose native recording ended with an error.

use std::path::Path;

use record_recovery::{read_manifest, recover_interrupted, OwnerState, RecoveryReceipt};

use super::ownership::{pause_session_ownership, PauseSessionOwnership};

/// Called by the capture worker itself, after its native session has ended and
/// before it reports the failure. The worker is the owner, so the capture is
/// sealed now instead of waiting for a later process to find it: verified
/// checkpoints become `recovered.mp4`. Returns the receipt the journal holds afterwards, or
/// `None` when the capture belongs to a pause session (sealed by its own
/// owner) or the journal cannot be read.
pub(crate) fn seal_after_failure(capture_dir: &Path) -> Option<RecoveryReceipt> {
    if !matches!(
        pause_session_ownership(capture_dir),
        Ok(PauseSessionOwnership::Absent)
    ) {
        return None;
    }
    let manifest = read_manifest(capture_dir).ok()?;
    if let Some(receipt) = manifest.receipt {
        return Some(receipt);
    }
    // Nothing was saved yet: there is nothing to keep, and a failed capture
    // never gets a terminal receipt from its own failing worker. Its folder is
    // reported as not recoverable and sealed by the next scan or an explicit
    // screen_record.recover.
    if manifest.checkpoints.is_empty() {
        return None;
    }
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let ffprobe = cut_media::toolpath::ffprobe();
    match recover_interrupted(
        capture_dir,
        &ffmpeg.to_string_lossy(),
        &ffprobe.to_string_lossy(),
        OwnerState::Dead,
    ) {
        Ok(Some(result)) => Some(result.receipt),
        Ok(None) => read_manifest(capture_dir).ok()?.receipt,
        Err(error) => {
            tracing::warn!(error = %error, "failed capture could not be sealed; a later scan retries");
            None
        }
    }
}
