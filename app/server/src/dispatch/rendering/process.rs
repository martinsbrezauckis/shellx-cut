//! Server-to-media bridge for one cancellable final-render operation.

use crate::jobs::JobCancellation;
use cut_core::CutError;
use std::time::Duration;

// Long-form exports are legitimate, but no render may wait indefinitely. Every
// ffmpeg phase in one render shares this single deadline.
const RENDER_OPERATION_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

pub(super) fn run_owned_render<T>(
    cancellation: JobCancellation,
    render: impl FnOnce() -> Result<T, CutError>,
) -> Result<T, CutError> {
    let probe = cancellation.clone();
    let control =
        cut_media::ffmpeg::RenderProcessControl::bounded(RENDER_OPERATION_TIMEOUT, move || {
            probe.is_cancelled()
        });
    cut_media::ffmpeg::with_render_process_control(&control, render)
}
