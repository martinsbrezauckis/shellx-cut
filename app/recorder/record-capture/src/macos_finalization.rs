//! Exact macOS capture-end ordering for ScreenCaptureKit and Core Audio.
//!
//! The Core Audio process tap must stop at the same capture-clock boundary as
//! video. Stitching sparse checkpoints can be expensive on a 4K desktop, so
//! stopping the tap afterwards records fabricated tail time while ffmpeg works.

/// Stop video first, then detach the Core Audio tap before any checkpoint work.
///
/// This stays generic so its ordering contract is unit-tested without a TCC
/// prompt, an SCK stream, or a Core Audio device. The returned audio payload can
/// be published only after the video checkpoint/stitch path is durable.
pub(crate) fn stop_audio_at_video_boundary<T, R>(
    stop_video: impl FnOnce(),
    system_audio: &mut Option<T>,
    finish_audio: impl FnOnce(T) -> R,
) -> Option<R> {
    stop_video();
    system_audio.take().map(finish_audio)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::stop_audio_at_video_boundary;

    #[test]
    fn stops_video_and_audio_before_expensive_finalization() {
        let phases = RefCell::new(Vec::new());
        let mut tap = Some("tap");

        let audio = stop_audio_at_video_boundary(
            || phases.borrow_mut().push("video-stop"),
            &mut tap,
            |tap| {
                phases.borrow_mut().push("audio-stop");
                tap
            },
        );
        phases.borrow_mut().push("checkpoint-and-stitch");
        phases.borrow_mut().push("publish-wav");

        assert_eq!(audio, Some("tap"));
        assert!(tap.is_none());
        assert_eq!(
            phases.into_inner(),
            vec![
                "video-stop",
                "audio-stop",
                "checkpoint-and-stitch",
                "publish-wav"
            ]
        );
    }

    #[test]
    fn still_stops_video_when_system_audio_is_unavailable() {
        let phases = RefCell::new(Vec::new());
        let mut tap: Option<()> = None;

        let audio = stop_audio_at_video_boundary(
            || phases.borrow_mut().push("video-stop"),
            &mut tap,
            |_| phases.borrow_mut().push("audio-stop"),
        );

        assert_eq!(audio, None);
        assert_eq!(phases.into_inner(), vec!["video-stop"]);
    }
}
