//! Exact frame counts and the ffmpeg graph for one constant-rate part.
//!
//! Two lessons from signed test builds on macOS:
//! - Trimming a part by time left its last frame a fraction of a millisecond
//!   short (0.033008 s instead of 0.033333 s); after the concat the take's
//!   average rate was no longer exactly 30/1 and the person read a false
//!   "Saved with a problem: ... not steady" note. Parts are now cut by frame
//!   count, from boundaries rounded on the take's absolute clock.
//! - A long still stretch (the picture held for 17 to 43 s) was encoded frame
//!   by frame at Stop (17 to 51 s of Stop under load). The held time after the
//!   last real frame is now one short "hold unit" (about a second of the held
//!   picture), encoded once and repeated by the concat without re-encoding,
//!   the way a still is extended in a timeline. Same encoder and settings for
//!   the part and its unit, so both share one H.264 parameter set.

use super::plan::SegmentPlan;

/// How one part's frames are made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PartFrames {
    /// Frames of the whole part on the take's clock.
    pub(crate) total: u64,
    /// Frames encoded in the main file (lead, real frames, short tail).
    pub(crate) main: u64,
    /// Frames of the hold unit (0 when the part has no unit).
    pub(crate) unit: u64,
    /// How many times the unit follows the main file in the concat.
    pub(crate) units: u64,
}

/// The frame index at `ms` on a `num/den` grid, rounded to the nearest frame.
pub(crate) fn frame_at(ms: u64, num: u64, den: u64) -> u64 {
    let scale = u128::from(den) * 1_000;
    ((u128::from(ms) * u128::from(num) + scale / 2) / scale) as u64
}

/// Milliseconds that `frames` frames last on a `num/den` grid (rounded).
pub(crate) fn frames_ms(frames: u64, num: u64, den: u64) -> u64 {
    let num = u128::from(num.max(1));
    ((u128::from(frames) * u128::from(den) * 1_000 + num / 2) / num) as u64
}

#[cfg(test)]
pub(crate) fn part_frames(plan: &SegmentPlan, num: u64, den: u64) -> PartFrames {
    let seconds = compact_unit_seconds(
        std::slice::from_ref(plan),
        num,
        den,
        super::MAX_CONCAT_ENTRIES,
    )
    .unwrap_or(60);
    part_frames_with_unit(plan, num, den, seconds)
}

/// A wider held-picture unit reduces concat rows without changing the exact
/// output frame count. Normal parts keep the one-second unit.
pub(crate) fn part_frames_with_unit(
    plan: &SegmentPlan,
    num: u64,
    den: u64,
    unit_seconds: u64,
) -> PartFrames {
    let total = frame_at(plan.end_ms.max(plan.start_ms), num, den)
        .saturating_sub(frame_at(plan.start_ms, num, den))
        .max(1);
    // Frames that may carry a picture that is not the held last one.
    let content_ms = plan.lead_ms.saturating_add(plan.media_ms);
    let scale = u128::from(den) * 1_000;
    let content = ((u128::from(content_ms) * u128::from(num)).div_ceil(scale) as u64)
        .max(1)
        .min(total);
    let unit = (num.max(1) + den.max(1) / 2) / den.max(1);
    let unit = unit.max(1).saturating_mul(unit_seconds);
    let tail = total - content;
    let units = if tail >= 2 * unit { tail / unit } else { 0 };
    PartFrames {
        total,
        main: total - units * unit,
        unit: if units > 0 { unit } else { 0 },
        units,
    }
}

/// Select the shortest reusable unit that fits the complete take's row budget.
/// Sixty seconds is the largest unit we encode once, even for a seven-day gap.
pub(crate) fn compact_unit_seconds(
    plans: &[SegmentPlan],
    num: u64,
    den: u64,
    max_entries: u64,
) -> Option<u64> {
    (1..=60).find(|&seconds| {
        plans
            .iter()
            .try_fold(0u64, |entries, plan| {
                entries.checked_add(1 + part_frames_with_unit(plan, num, den, seconds).units)
            })
            .is_some_and(|entries| entries <= max_entries)
    })
}

/// The filter graph: `[pa]` is the main file, `[pu]` the hold unit when the
/// part has one. `fps` is the cadence filter (`fps=30/1:eof_action=pass`).
pub(crate) fn part_graph(
    plan: &SegmentPlan,
    frames: &PartFrames,
    fps: &str,
    num: u64,
    den: u64,
) -> String {
    // Lead and pad in whole frames, and timestamps rewritten as frame index /
    // rate: a lead of 23 ms padded by time left the first part off the frame
    // grid and the take's average rate read 30.0001 (real 4K folders of
    // an early 0.6.116 build). The pad assumes the source gave no frame at all: the
    // trims cut exact counts and ffmpeg stops once both outputs ended, so a
    // long pad costs nothing. A pad sized from the container duration came out
    // 1.5 s short on a segment whose last frame lasted 5.4 s (QuickTime paused
    // inside it, A4) and 13.9 s of real screen were replaced by a held picture.
    let lead = frame_at(plan.lead_ms, num, den);
    let pad = frames.main + frames.unit + (num + den - 1) / den.max(1);
    let grid = format!("setpts=N/({num}/{den})/TB");
    let padded =
        format!("[0:v]{fps},tpad=start={lead}:start_mode=clone:stop={pad}:stop_mode=clone");
    if frames.units == 0 {
        return format!("{padded},trim=end_frame={},{grid}[pa]", frames.main);
    }
    format!(
        "{padded},split=2[a][b];[a]trim=end_frame={main},{grid}[pa];[b]trim=start_frame={main}:end_frame={end},{grid}[pu]",
        main = frames.main,
        end = frames.main + frames.unit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(start_ms: u64, lead_ms: u64, span_ms: u64, hold_ms: u64, media_ms: u64) -> SegmentPlan {
        let end_ms = start_ms + lead_ms + span_ms + hold_ms;
        SegmentPlan {
            start_ms,
            end_ms,
            lead_ms,
            span_ms,
            hold_ms,
            media_ms,
        }
    }

    #[test]
    fn parts_tile_the_take_clock_without_drift() {
        // A real UI take of an early build: five parts whose time-trimmed sum made
        // the average rate 30044160/1001467.
        let parts = [
            plan(0, 23, 14_980, 1_214, 14_980),
            plan(16_217, 0, 14_065, 818, 14_065),
            plan(31_100, 0, 14_525, 594, 14_283),
            plan(46_219, 0, 14_697, 265, 14_697),
            plan(61_181, 0, 3_968, 0, 3_995),
        ];
        let total: u64 = parts.iter().map(|p| part_frames(p, 30, 1).total).sum();
        assert_eq!(total, frame_at(65_149, 30, 1));
        assert_eq!(frames_ms(total, 30, 1).abs_diff(65_149), 16);
    }

    #[test]
    fn a_long_still_hold_becomes_repeated_units_and_motion_does_not() {
        // A1 of the run: 0.85 s of pictures, then the screen held for 58 s.
        let still = part_frames(&plan(0, 17, 15_048, 43_296, 850), 30, 1);
        assert_eq!(still.unit, 30);
        assert!(still.main < 60, "{still:?}");
        assert_eq!(still.main + still.units * still.unit, still.total);
        // A moving segment with a 0.5 s restart gap: one file, no unit.
        let moving = part_frames(&plan(15_590, 0, 14_994, 480, 14_967), 30, 1);
        assert_eq!((moving.units, moving.unit), (0, 0));
        assert_eq!(moving.main, moving.total);
        // 29.97: the unit is 30 frames, the boundaries still tile.
        let ntsc = part_frames(&plan(0, 0, 1_000, 9_000, 900), 30_000, 1_001);
        assert_eq!(ntsc.unit, 30);
        assert_eq!(ntsc.main + ntsc.units * 30, ntsc.total);
    }

    #[test]
    fn a_received_seven_day_gap_does_not_expand_to_a_row_per_second() {
        let gap = plan(0, 0, 1_000, super::super::plan::MAX_TAKE_MS - 2_000, 1_000);
        let frames = part_frames(&gap, 30, 1);
        assert!(
            frames.units < super::super::MAX_CONCAT_ENTRIES,
            "{frames:?}"
        );
        assert!(frames.unit > 30, "the unit must be wider than a second");
        assert_eq!(frames.main + frames.units * frames.unit, frames.total);
    }

    #[test]
    fn many_long_holds_share_one_concat_budget() {
        let plans = [
            plan(0, 0, 1_000, 86_400_000, 1_000),
            plan(86_401_000, 0, 1_000, 86_400_000, 1_000),
            plan(172_802_000, 0, 1_000, 86_400_000, 1_000),
        ];
        let seconds = compact_unit_seconds(&plans, 30, 1, super::super::MAX_CONCAT_ENTRIES)
            .expect("three-day hold fits with wider reusable units");
        assert!(seconds > 1);
        let entries: u64 = plans
            .iter()
            .map(|plan| 1 + part_frames_with_unit(plan, 30, 1, seconds).units)
            .sum();
        assert!(entries <= super::super::MAX_CONCAT_ENTRIES);
    }

    #[test]
    fn the_graph_cuts_by_frames_and_names_the_unit_output() {
        let p = plan(0, 17, 15_048, 43_296, 850);
        let f = part_frames(&p, 30, 1);
        let graph = part_graph(&p, &f, "fps=30/1:eof_action=pass", 30, 1);
        assert!(graph.contains(&format!("trim=end_frame={}", f.main)));
        assert!(graph.contains("[pu]") && graph.contains("split=2"));
        let m = plan(15_590, 0, 14_994, 480, 14_967);
        let graph = part_graph(&m, &part_frames(&m, 30, 1), "fps=30/1", 30, 1);
        assert!(!graph.contains("[pu]") && graph.ends_with("[pa]"));
    }
}
