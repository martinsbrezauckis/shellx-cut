//! Checked mapping between WGC's QPC-based frame time and the input clock.

use std::time::{Duration, Instant};

const HNS_PER_SECOND: i128 = 10_000_000;
const HNS_PER_MS: i128 = 10_000;
// The observed PC4 static native bracket was 100 ns. One millisecond keeps
// origin uncertainty far below a 30 fps frame without depending on one
// scheduler-interrupted pairing attempt.
const MAX_BEST_BRACKET_HNS: i128 = HNS_PER_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QpcCaptureOrigin {
    hns: i128,
    bracket_hns: i128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirstFramePlacement {
    AtOrAfterStart { start_ms: u64 },
    BeforeStart { lead_hns: u64 },
}

impl QpcCaptureOrigin {
    /// `paired_elapsed` is sampled from the same Instant as the QPC bracket.
    /// It may be nonzero when a capture-wide clock opened before input startup.
    pub(crate) fn from_sample(
        frequency: i64,
        qpc_before: i64,
        qpc_after: i64,
        paired_elapsed: Duration,
    ) -> Result<Self, &'static str> {
        if frequency <= 0 || qpc_before < 0 || qpc_after < qpc_before {
            return Err("invalid QPC frequency or sample bracket");
        }
        let frequency = i128::from(frequency);
        let before = i128::from(qpc_before);
        let after = i128::from(qpc_after);
        let width_hns = (after - before)
            .checked_mul(HNS_PER_SECOND)
            .ok_or("QPC sample width overflow")?
            / frequency;
        let midpoint = before + (after - before) / 2;
        let sampled_hns = midpoint
            .checked_mul(HNS_PER_SECOND)
            .ok_or("QPC sample conversion overflow")?
            / frequency;
        let elapsed_hns = i128::try_from(paired_elapsed.as_nanos() / 100)
            .map_err(|_| "capture clock elapsed time overflow")?;
        let hns = sampled_hns
            .checked_sub(elapsed_hns)
            .ok_or("capture QPC origin overflow")?;
        if hns < 0 {
            return Err("capture QPC origin predates system start");
        }
        Ok(Self {
            hns,
            bracket_hns: width_hns,
        })
    }

    #[cfg(windows)]
    pub(crate) fn sample(capture_start: Instant) -> Result<Self, &'static str> {
        use windows::Win32::System::Performance::{
            QueryPerformanceCounter, QueryPerformanceFrequency,
        };
        let mut frequency = 0i64;
        unsafe { QueryPerformanceFrequency(&mut frequency) }
            .map_err(|_| "QPC frequency query failed")?;
        let mut best: Option<Self> = None;
        // A scheduler interruption can widen one bracket. Each candidate is
        // paired locally with the same capture Instant; choose the narrowest.
        for _ in 0..7 {
            let mut before = 0i64;
            let mut after = 0i64;
            unsafe { QueryPerformanceCounter(&mut before) }
                .map_err(|_| "QPC before-sample failed")?;
            let paired_elapsed = capture_start.elapsed();
            unsafe { QueryPerformanceCounter(&mut after) }
                .map_err(|_| "QPC after-sample failed")?;
            let candidate = Self::from_sample(frequency, before, after, paired_elapsed)?;
            if best.is_none_or(|prior| candidate.bracket_hns < prior.bracket_hns) {
                best = Some(candidate);
            }
        }
        let best = best.ok_or("QPC origin sample unavailable")?;
        best.within_budget()
    }

    fn within_budget(self) -> Result<Self, &'static str> {
        if self.bracket_hns > MAX_BEST_BRACKET_HNS {
            return Err("all paired QPC origin brackets exceed one millisecond");
        }
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn bracket_hns(self) -> i128 {
        self.bracket_hns
    }

    pub(crate) fn place_first_frame(
        self,
        first_native_hns: i64,
        reserved_start_ms: u64,
    ) -> Result<FirstFramePlacement, &'static str> {
        if first_native_hns < 0 {
            return Err("negative native frame QPC timestamp");
        }
        let boundary = i128::from(self.reserved_boundary_hns(reserved_start_ms)?);
        let delta = i128::from(first_native_hns)
            .checked_sub(self.hns)
            .ok_or("native frame QPC delta overflow")?;
        if i128::from(first_native_hns) < boundary {
            return Ok(FirstFramePlacement::BeforeStart {
                lead_hns: u64::try_from(boundary - i128::from(first_native_hns))
                    .map_err(|_| "pre-reservation native frame lead overflow")?,
            });
        }
        let start_ms = delta
            .checked_add(HNS_PER_MS - 1)
            .ok_or("native frame millisecond rounding overflow")?
            / HNS_PER_MS;
        Ok(FirstFramePlacement::AtOrAfterStart {
            start_ms: u64::try_from(start_ms)
                .map_err(|_| "native frame elapsed milliseconds overflow")?,
        })
    }

    pub(crate) fn reserved_boundary_hns(self, reserved_start_ms: u64) -> Result<i64, &'static str> {
        let reserved_hns = i128::from(reserved_start_ms)
            .checked_mul(HNS_PER_MS)
            .ok_or("reserved WGC boundary overflow")?;
        let boundary = self
            .hns
            .checked_add(reserved_hns)
            .ok_or("native WGC boundary overflow")?;
        i64::try_from(boundary).map_err(|_| "native WGC boundary exceeds TimeSpan range")
    }

    /// Stop and input share `capture_start`; this is the final media PTS for
    /// either an initial cached state or a later native first frame.
    pub(crate) fn stop_media_timestamp(
        self,
        capture_start: Instant,
        stop_at: Instant,
        media_start_native_hns: i64,
    ) -> Result<i64, &'static str> {
        let elapsed_hns =
            i128::try_from(stop_at.saturating_duration_since(capture_start).as_nanos() / 100)
                .map_err(|_| "capture Stop elapsed time overflow")?;
        let media_start_elapsed_hns = i128::from(media_start_native_hns)
            .checked_sub(self.hns)
            .ok_or("media start clock overflow")?;
        i64::try_from(
            elapsed_hns
                .checked_sub(media_start_elapsed_hns)
                .ok_or("media Stop clock overflow")?,
        )
        .map_err(|_| "media Stop timestamp exceeds TimeSpan range")
    }
}

#[cfg(test)]
mod tests {
    use super::{FirstFramePlacement, QpcCaptureOrigin};
    use std::time::{Duration, Instant};

    #[test]
    fn retained_click_witness_uses_frame_qpc_not_callback_latency() {
        // First WGC callback can arrive 563 ms after Start even when its
        // compositor frame was rendered 560 ms after Start.
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        assert_eq!(
            origin.place_first_frame(55_600_000, 0).unwrap(),
            FirstFramePlacement::AtOrAfterStart { start_ms: 560 }
        );
    }

    #[test]
    fn prestarted_capture_clock_is_subtracted_before_native_placement() {
        let origin = QpcCaptureOrigin::from_sample(
            10_000_000,
            70_000_000,
            70_000_000,
            Duration::from_millis(400),
        )
        .unwrap();
        assert_eq!(
            origin.place_first_frame(72_100_001, 0).unwrap(),
            FirstFramePlacement::AtOrAfterStart { start_ms: 611 }
        );
    }

    #[test]
    fn frequency_conversion_and_pre_origin_state_are_explicit() {
        let origin =
            QpcCaptureOrigin::from_sample(2_000_000, 20_000_000, 20_000_000, Duration::ZERO)
                .unwrap();
        assert_eq!(
            origin.place_first_frame(99_000_000, 0).unwrap(),
            FirstFramePlacement::BeforeStart {
                lead_hns: 1_000_000
            }
        );
        assert_eq!(
            origin.place_first_frame(100_000_000, 0).unwrap(),
            FirstFramePlacement::AtOrAfterStart { start_ms: 0 }
        );
        assert_eq!(origin.reserved_boundary_hns(15_000), Ok(250_000_000));
        assert!(origin.reserved_boundary_hns(u64::MAX).is_err());
    }

    #[test]
    fn invalid_counter_samples_cannot_become_a_published_offset() {
        assert!(QpcCaptureOrigin::from_sample(0, 1, 1, Duration::ZERO).is_err());
        assert!(QpcCaptureOrigin::from_sample(10_000_000, -1, 1, Duration::ZERO).is_err());
        assert!(QpcCaptureOrigin::from_sample(10_000_000, 2, 1, Duration::ZERO).is_err());
        assert_eq!(
            QpcCaptureOrigin::from_sample(10_000_000, 1, 20_001, Duration::ZERO)
                .unwrap()
                .bracket_hns(),
            20_000
        );
        assert!(
            QpcCaptureOrigin::from_sample(10_000_000, 1, 20_001, Duration::ZERO)
                .unwrap()
                .within_budget()
                .is_err()
        );
        assert!(QpcCaptureOrigin::from_sample(10_000_000, 1, 1, Duration::from_secs(1)).is_err());
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        assert!(origin.place_first_frame(-1, 0).is_err());
        assert_eq!(
            origin.place_first_frame(50_010_000, 10).unwrap(),
            FirstFramePlacement::BeforeStart { lead_hns: 90_000 }
        );
    }

    #[test]
    fn cached_first_delayed_callback_still_holds_pixels_to_true_stop() {
        let start = Instant::now();
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        let reserved = origin.reserved_boundary_hns(0).unwrap();
        assert_eq!(
            origin.place_first_frame(reserved - 100_000, 0).unwrap(),
            FirstFramePlacement::BeforeStart { lead_hns: 100_000 }
        );
        // Callback arrives 500 ms after capture began, but the held surface
        // must extend to the actual 3.5 s Stop boundary, not just 3.0 s.
        let delayed_callback = start + Duration::from_millis(500);
        let stop = start + Duration::from_millis(3_500);
        assert_eq!(stop.duration_since(delayed_callback).as_millis(), 3_000);
        assert_eq!(
            origin.stop_media_timestamp(start, stop, reserved).unwrap(),
            35_000_000
        );
    }

    #[test]
    fn positive_first_delayed_callback_uses_native_media_origin_at_stop() {
        let start = Instant::now();
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        let first = 55_600_000;
        assert_eq!(
            origin.place_first_frame(first, 0).unwrap(),
            FirstFramePlacement::AtOrAfterStart { start_ms: 560 }
        );
        assert_eq!(
            origin
                .stop_media_timestamp(start, start + Duration::from_millis(3_500), first)
                .unwrap(),
            29_400_000
        );
    }

    #[test]
    fn later_cached_segment_ends_at_its_owned_reserved_boundary() {
        let start = Instant::now();
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        let second_boundary = origin.reserved_boundary_hns(15_000).unwrap();
        assert_eq!(
            origin
                .place_first_frame(second_boundary - 50_000, 15_000)
                .unwrap(),
            FirstFramePlacement::BeforeStart { lead_hns: 50_000 }
        );
        assert_eq!(
            origin
                .stop_media_timestamp(
                    start,
                    start + Duration::from_millis(18_000),
                    second_boundary
                )
                .unwrap(),
            30_000_000
        );
    }
}
