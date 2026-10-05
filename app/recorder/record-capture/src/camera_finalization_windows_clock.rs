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
    let numerator = ticks.checked_mul(HNS_PER_SECOND)?;
    // SourceReader converts each timestamp and duration separately. An exact
    // track tick can land between two 100-nanosecond values.
    if hns != numerator / scale && hns != numerator.div_ceil(scale) {
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
}
