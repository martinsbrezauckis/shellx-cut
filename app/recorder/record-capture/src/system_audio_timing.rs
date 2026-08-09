//! Exact first-packet timing for native system-loopback capture.
//!
//! A native backend can take a while to deliver its first real packet. The WAV
//! therefore starts at that packet, and this tracker records its offset from the
//! shared recorder clock. It deliberately never fills an elapsed gap with
//! synthetic samples.

#[cfg(any(not(target_os = "macos"), test))]
use std::time::Duration;

#[cfg(any(not(target_os = "macos"), test))]
use record_core::{error_codes, RecordError, Result};

/// Metadata returned with a finalized system-audio WAV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAudioCapture {
    pub path: String,
    /// `None` means the native backend stopped without delivering a packet.
    pub first_packet_offset_ms: Option<u64>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Debug, Default)]
pub(crate) struct SystemAudioTimingTracker {
    first_packet_offset_ms: Option<u64>,
    written_samples: usize,
}

#[cfg(any(not(target_os = "macos"), test))]
impl SystemAudioTimingTracker {
    /// Record one actual native-capture packet. `packet_samples` is never inferred from
    /// elapsed wall time, so a delayed packet cannot manufacture leading or tail
    /// silence in the persisted WAV.
    pub(crate) fn record_packet(&mut self, packet_samples: usize, elapsed: Duration) -> Result<()> {
        if packet_samples == 0 {
            return Ok(());
        }
        self.first_packet_offset_ms
            .get_or_insert_with(|| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        self.written_samples = self
            .written_samples
            .checked_add(packet_samples)
            .ok_or_else(|| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "write system audio",
                    "system-audio sample count overflowed",
                )
            })?;
        Ok(())
    }

    pub(crate) fn first_packet_offset_ms(&self) -> Option<u64> {
        self.first_packet_offset_ms
    }

    #[cfg(any(test, windows))]
    pub(crate) fn written_samples(&self) -> usize {
        self.written_samples
    }
}

#[cfg(test)]
mod tests {
    use super::SystemAudioTimingTracker;
    use std::time::Duration;

    #[test]
    fn first_packet_offsets_cover_zero_small_and_multi_second_delays() {
        for (delay, expected) in [
            (Duration::ZERO, Some(0)),
            (Duration::from_millis(37), Some(37)),
            (Duration::from_millis(2_350), Some(2_350)),
        ] {
            let mut tracker = SystemAudioTimingTracker::default();
            tracker.record_packet(960, delay).unwrap();
            assert_eq!(tracker.first_packet_offset_ms(), expected);
        }
    }

    #[test]
    fn delayed_packets_persist_only_actual_samples_without_padding() {
        let mut tracker = SystemAudioTimingTracker::default();
        tracker
            .record_packet(960, Duration::from_millis(2_000))
            .unwrap();
        tracker
            .record_packet(480, Duration::from_millis(2_010))
            .unwrap();

        assert_eq!(tracker.first_packet_offset_ms(), Some(2_000));
        assert_eq!(tracker.written_samples(), 1_440);
        // A later clock observation is not a packet and cannot add samples.
        assert_eq!(tracker.written_samples(), 1_440);
    }
}
