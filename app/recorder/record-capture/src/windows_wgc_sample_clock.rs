//! WGC sample time comes from native elapsed time, never callback count.

#[derive(Default)]
pub(crate) struct WgcSampleClock {
    first: Option<i64>,
    last: Option<i64>,
}

impl WgcSampleClock {
    pub(crate) fn timestamp(&mut self, native: i64) -> Result<i64, &'static str> {
        if self.last.is_some_and(|last| native <= last) {
            return Err("WGC video timestamps must strictly increase");
        }
        let first = self.first.unwrap_or(native);
        let elapsed = native
            .checked_sub(first)
            .ok_or("WGC video elapsed timestamp overflow")?;
        self.first = Some(first);
        self.last = Some(native);
        Ok(elapsed)
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
}
