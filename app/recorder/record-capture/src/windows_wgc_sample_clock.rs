//! WGC sample time comes from native elapsed time, never callback count.

use std::time::Instant;

use crate::windows_wgc_clock_origin::QpcCaptureOrigin;

#[derive(Default)]
pub(crate) struct WgcSampleClock {
    first: Option<i64>,
    last: Option<i64>,
    reserved_boundary: Option<i64>,
    cached_first: bool,
}

impl WgcSampleClock {
    /// Keep a cached first picture at PTS zero, but map later real compositor
    /// updates to the checkpoint's reserved capture-clock boundary.
    pub(crate) fn with_reserved_boundary(native_hns: i64) -> Self {
        Self {
            reserved_boundary: Some(native_hns),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn timestamp(&mut self, native: i64) -> Result<i64, &'static str> {
        self.admit_timestamp(native)?
            .ok_or("a pre-boundary WGC frame has no positive media timestamp")
    }

    /// `None` means the first cached state was already retained at PTS zero;
    /// another pre-boundary callback has no truthful later timestamp to encode.
    pub(crate) fn admit_timestamp(&mut self, native: i64) -> Result<Option<i64>, &'static str> {
        if self.reserved_boundary.is_some() && native < 0 {
            return Err("negative native frame QPC timestamp");
        }
        if self.last.is_some_and(|last| native <= last) {
            return Err("WGC video timestamps must strictly increase");
        }
        if self.first.is_none() {
            self.cached_first = self
                .reserved_boundary
                .is_some_and(|boundary| native < boundary);
        }
        let first = self.first.unwrap_or_else(|| {
            if self.cached_first {
                self.reserved_boundary
                    .expect("cached first needs a boundary")
            } else {
                native
            }
        });
        if self.cached_first && self.last.is_none() {
            self.first = Some(first);
            self.last = Some(native);
            return Ok(Some(0));
        }
        if self.cached_first && self.last.is_some() && native <= first {
            self.last = Some(native);
            return Ok(None);
        }
        let elapsed = native
            .checked_sub(first)
            .ok_or("WGC video elapsed timestamp overflow")?;
        self.first = Some(first);
        self.last = Some(native);
        Ok(Some(elapsed))
    }

    pub(crate) fn media_start_native_hns(&self) -> Option<i64> {
        self.first
    }

    /// The ordinary timed path ends at the same capture Instant as input;
    /// legacy pixel-only and pilot paths retain their callback-relative hold.
    pub(crate) fn held_at_stop(
        &self,
        last_timestamp: i64,
        accepted_at: Instant,
        stop_at: Instant,
        timed_origin: Option<(Instant, QpcCaptureOrigin)>,
    ) -> Result<(i64, i64), &'static str> {
        if let Some((capture_start, origin)) = timed_origin {
            let media_start = self
                .media_start_native_hns()
                .ok_or("WGC media start missing at owned Stop")?;
            let timestamp = origin.stop_media_timestamp(capture_start, stop_at, media_start)?;
            let elapsed = timestamp
                .checked_sub(last_timestamp)
                .ok_or("WGC held pixel interval overflow")?;
            Ok((timestamp, elapsed))
        } else {
            let elapsed = stop_at.saturating_duration_since(accepted_at);
            let elapsed_100ns = i64::try_from(elapsed.as_nanos() / 100)
                .map_err(|_| "WGC held pixel interval overflow")?;
            Ok((
                Self::held_timestamp(last_timestamp, elapsed_100ns)?,
                elapsed_100ns,
            ))
        }
    }

    /// Map an observed still-pixel interval onto the same native sample clock.
    /// The caller supplies the last accepted sample's elapsed timestamp and
    /// callback observation; no callback count or requested rate enters this.
    pub(crate) fn held_timestamp(
        last_timestamp: i64,
        elapsed_100ns: i64,
    ) -> Result<i64, &'static str> {
        if elapsed_100ns <= 0 {
            return Err("WGC held pixel interval must be positive");
        }
        last_timestamp
            .checked_add(elapsed_100ns)
            .ok_or("WGC held pixel timestamp overflow")
    }
}

#[cfg(test)]
mod tests {
    use super::WgcSampleClock;
    use crate::windows_wgc_clock_origin::QpcCaptureOrigin;
    use std::time::{Duration, Instant};

    #[test]
    fn requested_24_stream_keeps_native_refresh_and_stall_times() {
        let mut clock = WgcSampleClock::default();
        let origin = 88_000_000;
        // Refresh-driven callbacks can be 60 Hz even for a 24 fps encoder.
        assert_eq!(clock.timestamp(origin), Ok(0));
        assert_eq!(clock.timestamp(origin + 166_667), Ok(166_667));
        assert_eq!(clock.timestamp(origin + 333_334), Ok(333_334));
        // A 3-second native gap remains 3 seconds, not one 24 fps frame.
        assert_eq!(clock.timestamp(origin + 30_333_334), Ok(30_333_334));
    }

    #[test]
    fn rejected_regressions_cannot_rebase_the_segment_clock() {
        let mut clock = WgcSampleClock::default();
        assert_eq!(clock.timestamp(100), Ok(0));
        assert_eq!(clock.timestamp(200), Ok(100));
        assert!(clock.timestamp(200).is_err());
        assert!(clock.timestamp(150).is_err());
        assert_eq!(clock.timestamp(300), Ok(200));
        let mut next_segment = WgcSampleClock::default();
        assert_eq!(next_segment.timestamp(300), Ok(0));
    }

    #[test]
    fn elapsed_overflow_is_rejected_without_advancing_clock() {
        let mut clock = WgcSampleClock::default();
        assert_eq!(clock.timestamp(i64::MIN), Ok(0));
        assert!(clock.timestamp(i64::MAX).is_err());
        assert_eq!(clock.timestamp(i64::MIN + 1), Ok(1));
    }

    #[test]
    fn one_native_frame_can_end_at_observed_static_stop_time() {
        let mut clock = WgcSampleClock::default();
        let first = clock.timestamp(5_112_895_216_850).unwrap();
        assert_eq!(first, 0);
        assert_eq!(
            WgcSampleClock::held_timestamp(first, 145_791_450),
            Ok(145_791_450)
        );
        assert!(WgcSampleClock::held_timestamp(first, 0).is_err());
        assert!(WgcSampleClock::held_timestamp(i64::MAX, 1).is_err());
    }

    #[test]
    fn cached_first_is_visible_at_zero_then_updates_follow_reserved_qpc() {
        let mut clock = WgcSampleClock::with_reserved_boundary(1_000_000);
        assert!(clock.admit_timestamp(-1).is_err());
        assert_eq!(clock.admit_timestamp(900_000), Ok(Some(0)));
        assert_eq!(clock.media_start_native_hns(), Some(1_000_000));
        // Additional pre-start compositor states cannot be assigned invented
        // positive media time or replace the already admitted initial state.
        assert_eq!(clock.admit_timestamp(950_000), Ok(None));
        assert_eq!(clock.admit_timestamp(1_000_000), Ok(None));
        assert_eq!(clock.admit_timestamp(1_100_000), Ok(Some(100_000)));
        assert_eq!(clock.admit_timestamp(1_300_000), Ok(Some(300_000)));
    }

    #[test]
    fn positive_first_uses_first_frame_origin_for_external_leading_gap() {
        let mut clock = WgcSampleClock::with_reserved_boundary(1_000_000);
        assert_eq!(clock.admit_timestamp(1_560_000), Ok(Some(0)));
        assert_eq!(clock.media_start_native_hns(), Some(1_560_000));
        assert_eq!(clock.admit_timestamp(2_000_000), Ok(Some(440_000)));
    }

    #[test]
    fn each_physical_checkpoint_uses_its_own_reserved_boundary() {
        let capture_origin = 80_000_000i64;
        let second_reserved_ms = 15_000i64;
        let mut second =
            WgcSampleClock::with_reserved_boundary(capture_origin + second_reserved_ms * 10_000);
        assert_eq!(second.admit_timestamp(229_500_000), Ok(Some(0)));
        assert_eq!(second.admit_timestamp(230_100_000), Ok(Some(100_000)));
        assert!(second.admit_timestamp(230_100_000).is_err());
    }

    #[test]
    fn timed_hold_uses_capture_stop_while_legacy_hold_uses_callback() {
        let capture_start = Instant::now();
        let origin =
            QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO)
                .unwrap();
        let mut clock = WgcSampleClock::with_reserved_boundary(50_000_000);
        assert_eq!(clock.admit_timestamp(49_900_000), Ok(Some(0)));
        let callback = capture_start + Duration::from_millis(500);
        let stop = capture_start + Duration::from_millis(3_500);
        assert_eq!(
            clock.held_at_stop(0, callback, stop, Some((capture_start, origin))),
            Ok((35_000_000, 35_000_000))
        );
        assert_eq!(
            clock.held_at_stop(0, callback, stop, None),
            Ok((30_000_000, 30_000_000))
        );
    }
}
