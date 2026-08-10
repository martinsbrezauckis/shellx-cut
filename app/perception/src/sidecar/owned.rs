//! Owned sidecar adapters for server jobs and render stages.
//!
//! This leaf accepts `cut-media`'s process owner directly, keeping the
//! perception contract independent of the server job crate while ensuring the
//! Python child, its descendants, stdin writer, and diagnostic pipes finish
//! inside the caller's cancellation/deadline budget.

use super::*;

/// Run instruments under one caller-owned operation control.
pub fn run_instruments_owned(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<PerceptionReport, CutError> {
    run_instruments_owned_progress(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        None,
        control,
    )
}

/// Progress-aware owned variant of [`super::run_instruments`].
pub fn run_instruments_owned_progress(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    progress: Option<Arc<SidecarProgress>>,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<PerceptionReport, CutError> {
    run_instruments_with_owner(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        progress,
        Some(control),
        true,
    )
}

/// Run a caller-owned instrument pass without reading or writing the normal
/// project perception cache. Historic render verification publishes its own
/// immutable receipt and must not create a predictable temporary cache leaf.
pub fn run_instruments_owned_ephemeral(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<PerceptionReport, CutError> {
    run_instruments_with_owner(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        None,
        Some(control),
        false,
    )
}

/// Job-owned subject analysis.
pub fn run_subject_owned(
    media_path: &Path,
    asset_hash: &str,
    preset: &str,
    direction: Option<serde_json::Value>,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<crate::types::SubjectTrack, CutError> {
    run_subject_with_owner(media_path, asset_hash, preset, direction, Some(control))
}

pub(super) fn run_subject_with_owner(
    media_path: &Path,
    asset_hash: &str,
    preset: &str,
    direction: Option<serde_json::Value>,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
) -> Result<crate::types::SubjectTrack, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-subject",
        "asset_hash": asset_hash,
        "instruments": ["subject"],
        "subject_preset": preset,
        "direction": direction,
    });
    tracing::info!(
        preset,
        directed = direction.is_some(),
        "running subject instrument"
    );
    let stdout = spawn_with_optional_owner(&request, control)?;
    let report: PerceptionReport = serde_json::from_str(stdout.trim())
        .map_err(|e| invalid_json("sidecar emitted invalid PerceptionReport JSON", &stdout, e))?;
    if report.schema != PERCEPTION_SCHEMA {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "sidecar emitted wrong schema",
            format!("found '{}', expected '{PERCEPTION_SCHEMA}'", report.schema),
        ));
    }
    report.subject_track.ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "subject instrument produced no track",
            "the perception sidecar may be missing the CV deps (torchvision/supervision)",
        )
    })
}

/// Job-owned contact-sheet generation.
pub fn build_contact_sheet_owned(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<serde_json::Value, CutError> {
    build_contact_sheet_with_owner(media_path, out_dir, preset, Some(control))
}

pub(super) fn build_contact_sheet_with_owner(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
) -> Result<serde_json::Value, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-contact",
        "asset_hash": "",
        "contact_sheet": out_dir,
        "subject_preset": preset,
    });
    tracing::info!(preset, "building director contact sheet");
    let stdout = spawn_with_optional_owner(&request, control)?;
    serde_json::from_str(stdout.trim())
        .map_err(|e| invalid_json("contact-sheet sidecar emitted invalid JSON", &stdout, e))
}

/// Job-owned output QC-sheet generation.
pub fn build_qc_sheet_owned(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
    control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<serde_json::Value, CutError> {
    build_qc_sheet_with_owner(media_path, out_dir, preset, Some(control))
}

pub(super) fn build_qc_sheet_with_owner(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
) -> Result<serde_json::Value, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-qc",
        "asset_hash": "",
        "qc_sheet": out_dir,
        "subject_preset": preset,
    });
    tracing::info!(preset, "building director QC sheet");
    let stdout = spawn_with_optional_owner(&request, control)?;
    serde_json::from_str(stdout.trim())
        .map_err(|e| invalid_json("qc-sheet sidecar emitted invalid JSON", &stdout, e))
}

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
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&transcript).map_err(CutError::from)?,
    )?;
    Ok(transcript)
}

fn spawn_with_optional_owner(
    request: &serde_json::Value,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
) -> Result<String, CutError> {
    match control {
        Some(control) => spawn_sidecar_streaming(request, None, Some(control)),
        None => spawn_sidecar(request),
    }
}

fn invalid_json(message: &str, stdout: &str, error: serde_json::Error) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        message,
        format!(
            "{error}; first 200 chars: {}",
            stdout.chars().take(200).collect::<String>()
        ),
    )
}
