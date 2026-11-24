//! Exact conversion between Media Foundation units and the MP4 track clock.
//!
//! Container endpoints are quantized independently. Their span can therefore
//! occupy either adjacent track tick of the native interval. Admission uses
//! those integer ticks, never a millisecond or frame-size tolerance.

const HNS_PER_SECOND: u128 = 10_000_000;

pub(super) fn sample_ticks(hns: i64, timescale: u32) -> Option<u64> {
    if timescale == 0 || u128::from(timescale) > HNS_PER_SECOND {
        return None;
    }
    let hns = u128::try_from(hns).ok()?;
    let scale = u128::from(timescale);
    let ticks = (hns.checked_mul(scale)? + HNS_PER_SECOND / 2) / HNS_PER_SECOND;
    if ticks == 0 {
        return (hns == 0).then_some(0);
    }
    let numerator = ticks.checked_mul(HNS_PER_SECOND)?;
    // SourceReader converts each timestamp and duration separately. With ctts
    // reordering, it converts dts and ctts to 100-ns integers separately and
    // takes differences between converted starts, adding at most 1 hns of
    // split-floor drift when scale does not divide 10^7 and ticks are >=4 hns apart.
    let split_slack =
        u128::from(!HNS_PER_SECOND.is_multiple_of(scale) && scale <= HNS_PER_SECOND / 4);
    let min_hns = (numerator / scale).saturating_sub(split_slack);
    let max_hns = numerator.div_ceil(scale) + split_slack;
    if hns < min_hns || hns > max_hns {
        return None;
    }
    u64::try_from(ticks).ok()
}

pub(super) fn span_matches(native_hns: i64, encoded_ticks: u64, timescale: u32) -> bool {
    if native_hns <= 0 || encoded_ticks == 0 || timescale == 0 {
        return false;
    }
    let scaled = native_hns as u128 * u128::from(timescale);
    let encoded = u128::from(encoded_ticks);
    encoded == scaled / HNS_PER_SECOND || encoded == scaled.div_ceil(HNS_PER_SECOND)
}

pub(super) fn reconciled_duration_ticks(
    measured_ticks: u64,
    final_pts_duration_ticks: u64,
    stts_total_ticks: u64,
    last_stts_delta: u64,
    has_ctts: bool,
    has_elst: bool,
) -> Option<u64> {
    if measured_ticks == 0 || stts_total_ticks == 0 {
        return None;
    }
    if measured_ticks == stts_total_ticks {
        return Some(measured_ticks);
    }
    // When Capture Engine emits reordered B-frames (ctts, no elst), it stores
    // the camera's nominal tail duration on the final DTS sample while
    // SourceReader synthesizes next_pts - cur_pts and leaves the highest-PTS
    // sample with an inter-frame delta: deficit == last_stts_delta - final_pts_dur.
    if has_ctts
        && !has_elst
        && stts_total_ticks > measured_ticks
        && last_stts_delta > final_pts_duration_ticks
        && stts_total_ticks - measured_ticks == last_stts_delta - final_pts_duration_ticks
    {
        return Some(stts_total_ticks);
    }
    None
}

pub(super) fn ticks_to_floor_hns(ticks: u64, timescale: u32) -> Option<i64> {
    if ticks == 0 || timescale == 0 {
        return None;
    }
    let hns = u128::from(ticks).checked_mul(HNS_PER_SECOND)? / u128::from(timescale);
    i64::try_from(hns).ok().filter(|value| *value > 0)
}

pub(super) struct MeasuredTrackSpan {
    pub(super) frame_count: u64,
    pub(super) duration_hns: i64,
    pub(super) duration_ticks: Option<u64>,
    pub(super) final_pts_duration_ticks: Option<u64>,
}

pub(super) fn reconcile_track_span(
    measured: MeasuredTrackSpan,
    track: &super::timescale::VideoTrackTiming,
) -> std::result::Result<(Option<u64>, i64), String> {
    if track.stts_samples != measured.frame_count {
        return Err(format!(
            "container sample count differs from decoded frames; stts_samples={}; decoded_frames={}",
            track.stts_samples, measured.frame_count
        ));
    }
    let (Some(raw_ticks), Some(final_pts_dur)) =
        (measured.duration_ticks, measured.final_pts_duration_ticks)
    else {
        return Ok((measured.duration_ticks, measured.duration_hns));
    };
    let reconciled = reconciled_duration_ticks(
        raw_ticks,
        final_pts_dur,
        track.stts_total_ticks,
        track.last_stts_delta,
        track.has_ctts,
        track.has_elst,
    )
    .ok_or_else(|| {
        format!(
            "decoded_ticks={raw_ticks}; stts_ticks={}; last_stts_delta={}; final_pts_dur={final_pts_dur}; has_ctts={}",
            track.stts_total_ticks, track.last_stts_delta, track.has_ctts
        )
    })?;
    if reconciled == raw_ticks {
        return Ok((Some(reconciled), measured.duration_hns));
    }
    let hns = ticks_to_floor_hns(reconciled, track.timescale)
        .ok_or_else(|| "container track ticks overflowed the Media Foundation span".to_owned())?;
    Ok((Some(reconciled), hns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_camera_spans_match_only_their_adjacent_container_ticks() {
        assert!(span_matches(87_388_184, 262_165, 30_000));
        assert!(span_matches(87_167_011, 261_501, 30_000));
        assert!(!span_matches(87_388_184, 262_166, 30_000));
        assert!(!span_matches(87_167_011, 261_500, 30_000));
        assert!(!span_matches(87_388_184, 263_665, 30_000));
    }

    #[test]
    fn separately_projected_sample_fields_recover_exact_track_ticks() {
        // Native readback: floor(last PTS) + floor(duration) = 87,166,999,
        // one hns below floor(their combined track span).
        let start = sample_ticks(86_833_666, 30_000).unwrap();
        let duration = sample_ticks(333_333, 30_000).unwrap();
        assert_eq!(start, 260_501);
        assert_eq!(duration, 1_000);
        assert_eq!(start + duration, 261_501);
        assert_eq!(sample_ticks(86_833_667, 30_000), Some(start));
    }

    #[test]
    fn nonrepresentable_decoded_times_and_invalid_clocks_are_rejected() {
        assert_eq!(sample_ticks(87_388_184, 30_000), None);
        assert_eq!(sample_ticks(-1, 30_000), None);
        assert_eq!(sample_ticks(100, 0), None);
        assert_eq!(sample_ticks(100, 10_000_001), None);
        assert!(!span_matches(0, 1, 30_000));
        assert!(!span_matches(1, 0, 30_000));
        assert!(!span_matches(1, 1, 0));
    }

    #[test]
    fn exact_and_fractional_tick_spans_preserve_both_endpoint_roundings() {
        for scale in [1_000_u32, 30_000, 90_000, 10_000_000] {
            for start in [0_u128, 17, 1_234_567] {
                for duration in [333_333_u128, 1_234_567, 80_000_001] {
                    let first = start * u128::from(scale) / HNS_PER_SECOND;
                    let end = (start + duration) * u128::from(scale) / HNS_PER_SECOND;
                    assert!(span_matches(duration as i64, (end - first) as u64, scale));
                }
            }
        }
    }

    #[test]
    fn ctts_split_floor_timestamps_and_durations_recover_exact_ticks() {
        // Observed SourceReader readback on Windows B-frame camera MP4s (30 kHz):
        // floor(dts_hns) + floor(ctts_hns) can land 1 hns below floor(pts_hns),
        // and next_pts_hns - cur_pts_hns can land 1 hns outside [floor, ceil].
        assert_eq!(sample_ticks(431_999, 30_000), Some(1_296));
        assert_eq!(sample_ticks(920_332, 30_000), Some(2_761));
        assert_eq!(sample_ticks(98_001, 30_000), Some(294));
        assert_eq!(sample_ticks(97_999, 30_000), Some(294));
        assert_eq!(sample_ticks(99_332, 30_000), Some(298));

        // Zero tick and exact-divisor timescales keep zero split slack; >=2 hns
        // drift remains rejected.
        assert_eq!(sample_ticks(1, 30_000), None);
        assert_eq!(sample_ticks(431_998, 30_000), None);
        assert_eq!(sample_ticks(98_002, 30_000), None);
        assert_eq!(sample_ticks(10_001, 1_000), None);
        assert_eq!(sample_ticks(9_999, 1_000), None);
        assert_eq!(sample_ticks(99, 5_000_000), None);
    }

    #[test]
    fn reordered_eos_duration_reconciles_missing_terminal_ctts_tail() {
        // Real 0.6.121 Windows B-frame takes (3, 4, 7, 9): SourceReader drops the
        // 1000-tick terminal delta on the trailing B-frame and leaves the
        // highest-PTS frame with an inter-frame delta (296 or 297 ticks).
        assert_eq!(
            reconciled_duration_ticks(350_654, 296, 351_358, 1_000, true, false),
            Some(351_358)
        );
        assert_eq!(
            reconciled_duration_ticks(353_772, 296, 354_476, 1_000, true, false),
            Some(354_476)
        );
        assert_eq!(
            reconciled_duration_ticks(352_039, 297, 352_742, 1_000, true, false),
            Some(352_742)
        );
        // Trailing P-frame (takes 5, 8) or non-ctts track (takes 2, 6, 10) matches directly.
        assert_eq!(
            reconciled_duration_ticks(358_668, 1_000, 358_668, 1_000, true, false),
            Some(358_668)
        );
        assert_eq!(
            reconciled_duration_ticks(352_258, 1_000, 352_258, 1_000, false, false),
            Some(352_258)
        );
        // Must reject without ctts, with elst, or with mismatched deficit.
        assert_eq!(
            reconciled_duration_ticks(350_654, 296, 351_358, 1_000, false, false),
            None
        );
        assert_eq!(
            reconciled_duration_ticks(350_654, 296, 351_358, 1_000, true, true),
            None
        );
        assert_eq!(
            reconciled_duration_ticks(350_653, 296, 351_358, 1_000, true, false),
            None
        );
        assert_eq!(ticks_to_floor_hns(351_358, 30_000), Some(117_119_333));
    }

    #[test]
    fn track_timing_summary_reads_stts_ctts_and_elst() {
        use super::super::timescale::{video_track_timing, VideoTrackTiming};
        use std::io::Cursor;

        fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut out = Vec::from(u32::try_from(body.len() + 8).unwrap().to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out
        }
        let mut mdhd = vec![0; 24];
        mdhd[12..16].copy_from_slice(&30_000u32.to_be_bytes());
        let mut hdlr = vec![0; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let mut stts = vec![0; 24];
        stts[4..8].copy_from_slice(&2u32.to_be_bytes());
        stts[8..12].copy_from_slice(&2u32.to_be_bytes());
        stts[12..16].copy_from_slice(&296u32.to_be_bytes());
        stts[16..20].copy_from_slice(&1u32.to_be_bytes());
        stts[20..24].copy_from_slice(&1_000u32.to_be_bytes());
        let mut ctts = vec![0; 8];
        ctts[4..8].copy_from_slice(&3u32.to_be_bytes());
        let mut stbl = atom(b"stts", &stts);
        stbl.extend(atom(b"ctts", &ctts));
        let mut mdia = atom(b"mdhd", &mdhd);
        mdia.extend(atom(b"hdlr", &hdlr));
        mdia.extend(atom(b"minf", &atom(b"stbl", &stbl)));
        let no_elst = atom(b"moov", &atom(b"trak", &atom(b"mdia", &mdia)));
        assert_eq!(
            video_track_timing(&mut Cursor::new(&no_elst), no_elst.len() as u64).unwrap(),
            VideoTrackTiming {
                timescale: 30_000,
                has_ctts: true,
                has_elst: false,
                stts_samples: 3,
                stts_total_ticks: 1_592,
                last_stts_delta: 1_000,
            }
        );
        let mut elst = vec![0; 8];
        elst[4..8].copy_from_slice(&1u32.to_be_bytes());
        let mut trak = atom(b"edts", &atom(b"elst", &elst));
        trak.extend(atom(b"mdia", &mdia));
        let with_elst = atom(b"moov", &atom(b"trak", &trak));
        assert!(
            video_track_timing(&mut Cursor::new(&with_elst), with_elst.len() as u64)
                .unwrap()
                .has_elst
        );
    }
}
