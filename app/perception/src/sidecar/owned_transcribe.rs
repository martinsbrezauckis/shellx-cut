//! Transcription under a caller-owned operation control (split from `owned`).

use super::*;

/// Progress-aware transcription under the caller's operation owner.
pub fn transcribe_owned_progress(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    model: Option<&str>,
    progress: Option<Arc<SidecarProgress>>,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<Transcript, CutError> {
    let report = run_instruments_owned_progress(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        InstrumentSet::WordsOnly,
        model,
        progress,
        control,
    )?;
    let transcript = transcript_or_empty(report, asset_id)?;
    let path = receipts_dir.join(format!("{asset_id}.words.json"));
    // Replaces the name: never writes through a link or shared file.
    cut_core::store::replace_project_file(
        &path,
        serde_json::to_string_pretty(&transcript)
            .map_err(CutError::from)?
            .as_bytes(),
    )?;
    Ok(transcript)
}
