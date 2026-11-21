//! Which screen frames reached the current ScreenCaptureKit recording output.
//!
//! ScreenCaptureKit delivers a frame only when the screen changes: a still
//! desktop gives a few frames and then none for as long as nothing moves. An
//! `SCRecordingOutput` that receives no frame at all cannot be finished ("failed
//! due to failure to process first sample buffer") and its file starts at its
//! first frame, not at the moment it was attached. The recorder therefore
//! needs, per output: whether any picture arrived (an empty output is never
//! rotated) and the capture-clock time of the first one (where the file's
//! first frame belongs on the take's timeline, so the picture and the sound
//! stay in sync after a long still stretch).

#[cfg(test)]
use std::sync::{Mutex, OnceLock};
#[cfg(test)]
use std::time::Instant;

/// Facts about the frames delivered since the current output was attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FrameWindow {
    pub(crate) frames: u64,
    /// Capture-clock milliseconds of the first delivered picture.
    pub(crate) first_ms: Option<u64>,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct SegmentFrames {
    origin: OnceLock<Instant>,
    window: Mutex<FrameWindow>,
}

#[cfg(test)]
impl SegmentFrames {
    /// The capture clock's zero. Frames delivered before it is known (during
    /// `start_capture`) count at time zero.
    pub(crate) fn set_origin(&self, origin: Instant) {
        let _ = self.origin.set(origin);
    }

    /// A new output is about to be attached: only frames from now on count.
    pub(crate) fn arm(&self) {
        *self.lock() = FrameWindow::default();
    }

    /// A frame carrying a picture was delivered at `at`.
    pub(crate) fn observe_picture(&self, at: Instant) {
        let ms = self.origin.get().map_or(0, |origin| {
            u64::try_from(at.saturating_duration_since(*origin).as_millis()).unwrap_or(u64::MAX)
        });
        let mut window = self.lock();
        window.frames = window.frames.saturating_add(1);
        window.first_ms.get_or_insert(ms);
    }

    pub(crate) fn window(&self) -> FrameWindow {
        *self.lock()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FrameWindow> {
        self.window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Where a finished output's first frame sits on the capture clock: its first
/// delivered picture, never before the previous segment ended and always
/// before this segment's end (the checkpoint journal needs `start < end`).
pub(crate) fn segment_start_ms(
    window: FrameWindow,
    attached_ms: u64,
    previous_end_ms: Option<u64>,
    end_ms: u64,
) -> u64 {
    let first = window.first_ms.unwrap_or(attached_ms).max(attached_ms);
    let first = previous_end_ms.map_or(first, |previous| first.max(previous));
    first.min(end_ms.saturating_sub(1))
}

/// At a rotation point: an output that has not received a picture is kept
/// running (detaching it would fail it). Stop always closes it.
pub(crate) fn keep_output_open(final_segment: bool, window: FrameWindow) -> bool {
    !final_segment && window.frames == 0
}

/// A closed output that could not be published as captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unfinished {
    /// Hold the previous segment's last frame from `start_ms` to the segment's
    /// end. `tell` is false for the ordinary still-screen case (no picture
    /// arrived, so nothing was lost) and true when pictures were lost.
    Hold { start_ms: u64, tell: bool },
    /// No earlier picture exists: there is no take to hold (an early Stop).
    Fail,
}

pub(crate) fn unfinished(previous_end_ms: Option<u64>, window: FrameWindow) -> Unfinished {
    match previous_end_ms {
        Some(start_ms) => Unfinished::Hold {
            start_ms,
            tell: window.frames > 0,
        },
        None => Unfinished::Fail,
    }
}

/// ScreenCaptureKit status values (`SCFrameStatus`) that carry no new
/// picture: idle, blank, suspended, stopped.
pub(crate) fn status_carries_picture(status: Result<i32, &'static str>, has_image: bool) -> bool {
    has_image && !matches!(status, Ok(1 | 2 | 3 | 5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_window_counts_only_frames_after_it_was_armed_and_keeps_the_first_time() {
        let frames = SegmentFrames::default();
        let origin = Instant::now();
        frames.observe_picture(origin); // during start_capture: before the origin is known
        frames.set_origin(origin);
        assert_eq!(
            frames.window(),
            FrameWindow {
                frames: 1,
                first_ms: Some(0)
            }
        );
        frames.arm();
        assert_eq!(frames.window(), FrameWindow::default());
        frames.observe_picture(origin + Duration::from_millis(16_900));
        frames.observe_picture(origin + Duration::from_millis(17_000));
        assert_eq!(
            frames.window(),
            FrameWindow {
                frames: 2,
                first_ms: Some(16_900)
            }
        );
    }

    #[test]
    fn a_segment_starts_at_its_first_picture_inside_its_own_span() {
        let window = |first_ms| FrameWindow {
            frames: 1,
            first_ms,
        };
        // A still screen: the first picture came 9 s after the output was attached.
        assert_eq!(
            segment_start_ms(window(Some(24_000)), 15_629, Some(15_051), 30_660),
            24_000
        );
        // No picture known: the attach time.
        assert_eq!(
            segment_start_ms(window(None), 15_629, Some(15_051), 30_660),
            15_629
        );
        // Never before the previous end, never at or after its own end.
        assert_eq!(
            segment_start_ms(window(Some(15_000)), 14_990, Some(15_051), 30_660),
            15_051
        );
        assert_eq!(
            segment_start_ms(window(Some(40_000)), 15_629, None, 30_660),
            30_659
        );
    }

    /// The still-screen take of the measurement (an early build and 0.6.115
    /// alike): 24 pictures in the first 0.85 s, one picture in the second
    /// segment, none after. Before, the third rotation detached an empty output,
    /// ScreenCaptureKit failed it and the capture ended at 46 s.
    #[test]
    fn a_still_screen_take_keeps_its_empty_output_and_holds_the_last_picture_at_stop() {
        let with = |frames, first_ms| FrameWindow { frames, first_ms };
        // 15 s: segment 0 has pictures: rotate and publish from its first picture.
        assert!(!keep_output_open(false, with(24, Some(0))));
        assert_eq!(segment_start_ms(with(24, Some(0)), 4, None, 15_051), 4);
        // 30 s: segment 1 got one picture at 22.1 s: rotate, start at that picture.
        assert!(!keep_output_open(false, with(1, Some(22_100))));
        assert_eq!(
            segment_start_ms(with(1, Some(22_100)), 15_629, Some(15_051), 30_660),
            22_100
        );
        // 45 s and 60 s: no picture: the output stays attached, no rotation.
        assert!(keep_output_open(false, with(0, None)));
        // Stop at 62 s with still no picture: ScreenCaptureKit cannot finish the
        // output; the previous last picture is held, silently (nothing was lost).
        assert!(!keep_output_open(true, with(0, None)));
        assert_eq!(
            unfinished(Some(30_660), with(0, None)),
            Unfinished::Hold {
                start_ms: 30_660,
                tell: false
            }
        );
        // An output with pictures that could not be finished is held and named.
        assert_eq!(
            unfinished(Some(30_660), with(12, Some(31_000))),
            Unfinished::Hold {
                start_ms: 30_660,
                tell: true
            }
        );
        // Stop before any segment was saved: nothing to hold.
        assert_eq!(unfinished(None, with(0, None)), Unfinished::Fail);
    }

    #[test]
    fn idle_and_blank_status_frames_are_not_pictures() {
        assert!(status_carries_picture(Ok(0), true));
        assert!(status_carries_picture(Ok(4), true));
        assert!(status_carries_picture(Err("missing"), true));
        for idle in [1, 2, 3, 5] {
            assert!(!status_carries_picture(Ok(idle), true));
        }
        assert!(!status_carries_picture(Ok(0), false));
    }
}
