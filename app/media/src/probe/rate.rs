//! Which frame rate a clip gives a new timeline.
//!
//! ffprobe reports two rates: `r_frame_rate`, the stream's nominal rate (a
//! phone's "60 fps"), and `avg_frame_rate`, frames divided by duration. For a
//! constant-rate file they agree. A phone records VARIABLE frame rate: it drops
//! or stretches frames under load, so a "60 fps" clip can average 58.591 fps.
//! A timeline at 58.591 fps renders files whose players report neither rate,
//! and the frames are re-timed as if evenly spaced. Editors set such a
//! timeline to the nominal (standard) rate instead; the renderer then places
//! each source frame by its real timestamp.

use super::MediaProbe;

/// Standard production rates, exact (NTSC rates as their 1000/1001 fractions).
const STANDARD_RATES: &[f64] = &[
    24_000.0 / 1001.0,
    24.0,
    25.0,
    30_000.0 / 1001.0,
    30.0,
    48.0,
    50.0,
    60_000.0 / 1001.0,
    60.0,
    100.0,
    120_000.0 / 1001.0,
    120.0,
    240.0,
];

/// Nominal and average rates differing by more than this are variable.
const VFR_RELATIVE_GAP: f64 = 0.005;

/// `"60/1"` -> 60.0; "0/0" and nonsense -> None.
pub(super) fn parse_rate(r: &str) -> Option<f64> {
    let (num, den) = r.split_once('/')?;
    let (num, den) = (num.parse::<f64>().ok()?, den.parse::<f64>().ok()?);
    (den > 0.0 && num > 0.0)
        .then(|| num / den)
        .filter(|f| f.is_finite() && *f > 0.0)
}

fn nearest_standard(rate: f64, within: f64) -> Option<f64> {
    STANDARD_RATES
        .iter()
        .copied()
        .min_by(|a, b| (a - rate).abs().total_cmp(&(b - rate).abs()))
        .filter(|s| (s - rate).abs() / s <= within)
}

/// True when the nominal and average rates disagree: a variable-frame-rate clip.
pub(super) fn is_variable(nominal: Option<f64>, average: Option<f64>) -> bool {
    match (nominal, average) {
        (Some(n), Some(a)) => (n - a).abs() / a > VFR_RELATIVE_GAP,
        _ => false,
    }
}

/// The frame rate a new timeline takes from this clip, and a sentence for the
/// person when the clip is variable frame rate. Constant-rate clips keep their
/// average rate exactly as before. A variable-rate clip takes its nominal rate
/// when that is a standard rate, else the standard rate nearest its average
/// (within 3 %), else its average.
pub fn timeline_rate(probe: &MediaProbe) -> Option<(f64, Option<String>)> {
    let average = probe.fps?;
    if !probe.variable_frame_rate {
        return Some((average, None));
    }
    let chosen = probe
        .nominal_fps
        .and_then(|n| nearest_standard(n, 0.001))
        .or_else(|| nearest_standard(average, 0.03))
        .unwrap_or(average);
    let note = format!(
        "This clip has a variable frame rate ({} fps on average{}), so the timeline uses {} fps.",
        fmt_rate(average),
        probe
            .nominal_fps
            .map(|n| format!(", {} fps nominal", fmt_rate(n)))
            .unwrap_or_default(),
        fmt_rate(chosen)
    );
    Some((chosen, Some(note)))
}

/// 60 -> "60", 59.94005994 -> "59.94", 58.5912 -> "58.59".
pub fn fmt_rate(rate: f64) -> String {
    let s = format!("{rate:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(nominal: Option<f64>, average: f64) -> MediaProbe {
        MediaProbe {
            kind: "video".into(),
            duration_ms: Some(13_313),
            width: Some(3840),
            height: Some(2160),
            fps: Some(average),
            nominal_fps: nominal,
            variable_frame_rate: is_variable(nominal, Some(average)),
            has_audio: true,
            audio_rate: Some(48_000),
            format: "mov,mp4 / hevc+aac".into(),
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_hq_phone_clip_gets_a_60_fps_timeline_and_a_note() {
        // r_frame_rate 60/1, avg_frame_rate 70200000/1198127 (58.591).
        let p = probe(Some(60.0), 70_200_000.0 / 1_198_127.0);
        assert!(p.variable_frame_rate);
        let (fps, note) = timeline_rate(&p).unwrap();
        assert_eq!(fps, 60.0);
        assert_eq!(
            note.as_deref(),
            Some("This clip has a variable frame rate (58.59 fps on average, 60 fps nominal), so the timeline uses 60 fps.")
        );
    }

    #[test]
    fn constant_rate_clips_keep_their_rate_unchanged() {
        for rate in [24_000.0 / 1001.0, 25.0, 30.0, 60.0, 12.5] {
            let p = probe(Some(rate), rate);
            assert!(!p.variable_frame_rate);
            assert_eq!(timeline_rate(&p), Some((rate, None)));
        }
    }

    #[test]
    fn an_odd_nominal_rate_snaps_the_average_to_a_standard_rate() {
        // Some recorders write a timebase as r_frame_rate (90000/1).
        let p = probe(Some(90_000.0), 29.6);
        let (fps, note) = timeline_rate(&p).unwrap();
        assert!((fps - 30.0).abs() < 1e-9 || (fps - 30_000.0 / 1001.0).abs() < 1e-9);
        assert!(note.is_some());
        // Nothing standard nearby: keep the average.
        let p = probe(Some(90_000.0), 37.0);
        assert_eq!(timeline_rate(&p).unwrap().0, 37.0);
    }
}
