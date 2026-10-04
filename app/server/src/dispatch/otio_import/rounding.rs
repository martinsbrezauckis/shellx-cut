//! Admit only bounded, exact Cut frame-rounding drift at a probed media EOF.

use super::*;

const MAX_EOF_ROUNDING_MS: u64 = 50;
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct RawFrameRange {
    start: i64,
    duration: i64,
    rate: f64,
}

impl RawFrameRange {
    fn integer_frame(node: &Value, pointer: &str, allow_zero: bool) -> Option<i64> {
        let value = node.pointer(pointer)?.as_f64()?;
        (value.is_finite()
            && value >= if allow_zero { 0.0 } else { 1.0 }
            && value < MAX_EXACT_F64_INTEGER
            && value.fract() == 0.0)
            .then_some(value as i64)
    }

    pub(super) fn from_item(item: &Value) -> Option<Self> {
        if !matches!(item.get("OTIO_SCHEMA")?.as_str()?, "Clip.1" | "Clip.2") {
            return None;
        }
        let start = Self::integer_frame(item, "/source_range/start_time/value", true)?;
        let duration = Self::integer_frame(item, "/source_range/duration/value", false)?;
        let start_rate = item.pointer("/source_range/start_time/rate")?.as_f64()?;
        let duration_rate = item.pointer("/source_range/duration/rate")?.as_f64()?;
        (start_rate.is_finite() && start_rate > 0.0 && start_rate == duration_rate).then_some(
            Self {
                start,
                duration,
                rate: start_rate,
            },
        )
    }
}

pub(super) fn raw_frame_ranges(raw: &Value) -> Vec<Vec<Option<RawFrameRange>>> {
    raw.pointer("/tracks/children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|track| {
            track
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(RawFrameRange::from_item)
                .collect()
        })
        .collect()
}

fn rounded_frame_ms(frames: i64, tb: &cut_export::Timebase) -> Option<u64> {
    let frames = u128::try_from(frames).ok()?;
    let num = u128::try_from(tb.num).ok()?;
    let den = u128::try_from(tb.den).ok()?;
    if num == 0 || den == 0 {
        return None;
    }
    let numerator = frames.checked_mul(den)?.checked_mul(1000)?;
    u64::try_from(numerator.checked_add(num / 2)? / num).ok()
}

/// In-range foreign clips retain their existing parser semantics. Only an
/// overrun may invoke this narrow, no-more-than-50-ms Cut rounding exception.
pub(super) fn clamp_source_end_for_otio_rounding(
    source_in: u64,
    requested_end: u64,
    media_duration: u64,
    source_fps: Option<f64>,
    raw: Option<RawFrameRange>,
) -> Option<u64> {
    if requested_end <= media_duration {
        return Some(requested_end);
    }
    if source_in >= media_duration
        || requested_end.checked_sub(media_duration)? > MAX_EOF_ROUNDING_MS
    {
        return None;
    }
    let raw = raw?;
    let fps = source_fps.filter(|fps| fps.is_finite() && *fps > 0.0 && *fps <= 240.0)?;
    let tb = cut_export::Timebase::from_fps(fps).ok()?;
    if tb.num <= 0 || tb.den <= 0 || raw.rate != tb.fps_f64() {
        return None;
    }
    let in_frames = tb.frames_from_ms(source_in).ok()?;
    let out_frames = tb.frames_from_ms(media_duration).ok()?;
    let duration_frames = out_frames.checked_sub(in_frames)?;
    if duration_frames <= 0 || raw.start != in_frames || raw.duration != duration_frames {
        return None;
    }
    let reconstructed =
        rounded_frame_ms(in_frames, &tb)?.checked_add(rounded_frame_ms(duration_frames, &tb)?)?;
    // The parser uses f64 RationalTime conversion and separately rounds start
    // and duration; 1 ms allows that reconstruction difference, never a 51 ms
    // source EOF overrun (checked above against the actual requested end).
    (reconstructed.abs_diff(requested_end) <= 1).then_some(media_duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_range(start: i64, duration: i64, rate: f64) -> Option<RawFrameRange> {
        Some(RawFrameRange {
            start,
            duration,
            rate,
        })
    }

    #[test]
    fn exact_cut_rounding_is_capped_at_actual_fifty_milliseconds() {
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(1.0), frame_range(0, 1, 1.0)),
            Some(950)
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 949, Some(1.0), frame_range(0, 1, 1.0)),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                0,
                11_000,
                10_600,
                Some(1.0),
                frame_range(0, 11, 1.0)
            ),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                0,
                10_900,
                10_000,
                Some(1.0),
                frame_range(0, 11, 1.0)
            ),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                1_000,
                58_167,
                58_162,
                Some(30.0),
                frame_range(30, 1715, 30.0)
            ),
            Some(58_162)
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                1_000,
                58_166,
                58_162,
                Some(30.0),
                frame_range(30, 1715, 30.0)
            ),
            Some(58_162)
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                1_000,
                58_165,
                58_162,
                Some(30.0),
                frame_range(30, 1715, 30.0)
            ),
            None
        );
        let ntsc = cut_export::Timebase::from_fps(29.97).unwrap();
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                1_001,
                14_715,
                14_708,
                Some(ntsc.fps_f64()),
                frame_range(30, 411, ntsc.fps_f64())
            ),
            Some(14_708),
            "NTSC export separately rounds the source start and duration"
        );
    }

    #[test]
    fn overrun_requires_matching_integral_cut_frames_and_safe_rate() {
        let valid = frame_range(0, 1, 1.0);
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, None, valid),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(0.0001), valid),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(f64::INFINITY), valid),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(30.0), valid),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(1.0), None),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, Some(1.0), frame_range(0, 2, 1.0)),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(950, 1000, 950, Some(1.0), valid),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(
                0,
                u64::MAX,
                u64::MAX - 25,
                Some(30.0),
                frame_range(0, 1, 30.0)
            ),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 1000, 950, None, None),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 900, 950, None, None),
            Some(900),
            "ordinary in-range foreign clips keep their current timing rules"
        );

        let fractional = json!({"OTIO_SCHEMA":"Clip.1","source_range":{"start_time":{"value":0,"rate":1},"duration":{"value":1.5,"rate":1}}});
        let mixed_rate = json!({"OTIO_SCHEMA":"Clip.1","source_range":{"start_time":{"value":0,"rate":1},"duration":{"value":1,"rate":2}}});
        assert!(RawFrameRange::from_item(&fractional).is_none());
        assert!(RawFrameRange::from_item(&mixed_rate).is_none());
    }
}
