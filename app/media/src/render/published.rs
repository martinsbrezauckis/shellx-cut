//! Measuring an export that is already published.
//!
//! Every render path publishes its file atomically (staging file renamed into
//! place) and then measures it (ffprobe) for the receipt. Once the file is in
//! place the export is complete. The measurement used to run under the job's
//! cancellation, so a stop that arrived in that short gap failed the render
//! with "render cancelled" while the finished file stayed: a complete export
//! the job then called cancelled, with no receipt. Now the measurement of a
//! published file is not cut short by a stop (it stays bounded in time), so
//! the job records what happened: the export completed.

use std::path::Path;
use std::time::Duration;

use cut_core::CutError;

use crate::probe::MediaProbe;

/// ffprobe on one finished file; generous, but never unbounded.
const MEASURE_LIMIT: Duration = Duration::from_secs(10 * 60);

pub(super) fn probe(path: &Path) -> Result<MediaProbe, CutError> {
    let control = crate::ffmpeg::RenderProcessControl::bounded(MEASURE_LIMIT, || false);
    crate::ffmpeg::with_render_process_control(&control, || crate::probe::probe(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_requested_before_measuring_does_not_fail_the_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("done.mp4");
        let made = std::process::Command::new(crate::ffmpeg::ffmpeg_bin())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=25:duration=1",
            ])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        // The render's own control: its job has been told to stop.
        let stopped =
            crate::ffmpeg::RenderProcessControl::bounded(Duration::from_secs(60), || true);
        let plain =
            crate::ffmpeg::with_render_process_control(&stopped, || crate::probe::probe(&clip));
        assert!(
            plain.is_err(),
            "under the stopped job's control ffprobe is refused"
        );
        let measured =
            crate::ffmpeg::with_render_process_control(&stopped, || probe(&clip)).unwrap();
        assert!(measured.duration_ms.is_some_and(|ms| ms > 0));
    }
}
