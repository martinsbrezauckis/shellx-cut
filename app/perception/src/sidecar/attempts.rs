//! Supplemental evidence: never substitute a record-write failure for the
//! existing analysis result/error. Cache hits return before asked() is called.
use super::*;
use crate::analysis_attempts::AttemptRecorder;

pub(super) fn run_instruments_with_owner(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    progress: Option<Arc<SidecarProgress>>,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
    persist: bool,
) -> Result<PerceptionReport, CutError> {
    let attempts = Arc::new(AttemptRecorder::new(
        receipts_dir,
        asset_id,
        asset_hash,
        set.names(),
        persist,
    ));
    let result = super::run_instruments_recorded(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        progress,
        control,
        persist,
        attempts.clone(),
    );
    if let Err(error) = &result {
        attempts.finish(Some(error));
    }
    result
}
