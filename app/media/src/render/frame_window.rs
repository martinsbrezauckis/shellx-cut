//! Which frame `render.frame` shows at a position, and which render window
//! holds it.
//!
//! The answer is the frame the export has on screen at `at_ms` (row 194): the
//! output frame `k` covers `[k / fps, (k + 1) / fps)`, so it is
//! `floor(at_ms * fps / 1000)`, with half a millisecond of tolerance because
//! positions are whole milliseconds and frame starts are not (a playhead put
//! on frame 1 at 30 fps reads 33 ms, before the frame's 33.33 ms start). The
//! last frame of the timeline is the latest a position can show.
//!
//! The window is the one whose frames (on the whole timeline's grid, see
//! `frame_budget`) include that frame; the frame graph then keeps the frame by
//! its index inside the window (`trim=start_frame`: a time would be rounded to
//! the stream's frame time base). Before, the graph kept "the first frame at or
//! after at_ms", which was the NEXT frame whenever a position fell inside a
//! frame.

use super::frame_budget::frame_at;

/// `(window, frame index inside it)` for `at_ms`. `windows` tile the
/// timeline; an empty list (or a frame past them) uses the whole timeline.
pub(super) fn frame_window(
    windows: &[(u64, u64)],
    at_ms: u64,
    fps: f64,
    duration_ms: u64,
) -> ((u64, u64), u64) {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let frame = frame_index(at_ms, fps, duration_ms);
    let window = windows
        .iter()
        .find(|(a, b)| frame_at(*a, fps) <= frame && frame < frame_at(*b, fps))
        .copied()
        .unwrap_or((0, duration_ms));
    (window, frame - frame_at(window.0, fps).min(frame))
}

/// The export frame on screen at `at_ms` (see the module doc).
pub(super) fn frame_index(at_ms: u64, fps: f64, duration_ms: u64) -> u64 {
    let last = frame_at(duration_ms, fps).saturating_sub(1);
    (((at_ms as f64 + 0.5) * fps / 1000.0).floor() as u64).min(last)
}

/// The fast scrub frame (a proxy seek, same rate only): the source position of
/// the source frame the export shows at output frame `k` of the segment, or
/// `None` when the segment converts its rate (the composed frame is used).
/// The proxy keeps the source's frames and timestamps; the seek keeps the
/// first frame at or after the position, so it is floored to the millisecond.
/// A source whose rate the probe does not state keeps the millisecond mapping
/// (`src_in + at_ms - start`).
pub(super) fn fast_source_ms(
    asset: &cut_core::Asset,
    seg: &cut_core::EdlSegment,
    k: u64,
    at_ms: u64,
    fps: f64,
) -> Option<u64> {
    let Some(sf) = super::source_fps(asset) else {
        return Some(seg.src_in_ms? + at_ms.saturating_sub(seg.timeline_in_ms));
    };
    if (sf - fps).abs() >= 0.01 {
        return None;
    }
    let j = k.checked_sub(frame_at(seg.timeline_in_ms, fps))? as f64;
    let first = (seg.src_in_ms? as f64 * sf / 1000.0 - 1e-6).ceil().max(0.0);
    Some(((first + j) * 1000.0 / sf + 1e-6).floor() as u64)
}

#[cfg(test)]
mod tests {
    use super::frame_window;

    #[test]
    fn a_position_shows_the_export_frame_on_screen_then() {
        let windows = [(0, 20000), (20000, 40000), (40000, 45000)];
        // Frame 599 covers 19 966.7 .. 20 000: the first window, its frame 599.
        assert_eq!(
            frame_window(&windows, 19967, 30.0, 45000),
            ((0, 20000), 599)
        );
        assert_eq!(
            frame_window(&windows, 19999, 30.0, 45000),
            ((0, 20000), 599)
        );
        assert_eq!(
            frame_window(&windows, 20000, 30.0, 45000),
            ((20000, 40000), 0)
        );
        // A playhead put on frame 1 reads 33 ms (frame 1 starts at 33.33 ms).
        assert_eq!(frame_window(&windows, 33, 30.0, 45000), ((0, 20000), 1));
        // Inside frame 750 (25 000 .. 25 033): frame 150 of the second window.
        assert_eq!(
            frame_window(&windows, 25010, 30.0, 45000),
            ((20000, 40000), 150)
        );
        // Past the last frame's start: the last frame (1349, 149 in the last window).
        assert_eq!(
            frame_window(&windows, 44999, 30.0, 45000),
            ((40000, 45000), 149)
        );
        // No windows (short timeline): the whole timeline.
        assert_eq!(frame_window(&[], 1234, 30.0, 5000), ((0, 5000), 37));
    }
}
