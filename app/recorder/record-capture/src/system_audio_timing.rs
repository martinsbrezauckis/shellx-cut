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

/// One Windows/native packet with clock metadata for gapless timeline alignment.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimedAudioPacket {
    pub(crate) samples: usize,
    pub(crate) qpc_position_100ns: u64,
    pub(crate) discontinuity: bool,
    pub(crate) is_silent: bool,
    pub(crate) elapsed_at_capture: Duration,
    pub(crate) packet_start_elapsed: Option<Duration>,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
}

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
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    stream_origin_qpc_100ns: Option<u64>,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    last_packet_end_qpc_100ns: Option<u64>,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    first_packet_start_elapsed: Option<Duration>,
}

#[cfg(any(not(target_os = "macos"), test))]
impl SystemAudioTimingTracker {
    /// Record one actual native-capture packet. `packet_samples` is never inferred from
    /// elapsed wall time, so a delayed packet cannot manufacture leading or tail
    /// silence in the persisted WAV.
    #[cfg_attr(
        windows,
        allow(
            dead_code,
            reason = "Linux PipeWire capture records non-QPC packet arrivals here"
        )
    )]
    pub(crate) fn record_packet(&mut self, packet_samples: usize, elapsed: Duration) -> Result<()> {
        if packet_samples == 0 {
            return Ok(());
        }
        self.first_packet_offset_ms
            .get_or_insert_with(|| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        self.add_samples(packet_samples)
    }

    /// Record a timed Windows loopback packet, returning how many silent
    /// alignment samples must precede it to keep intermittent audio in sync.
    #[cfg(any(windows, test))]
    pub(crate) fn record_timed_packet(&mut self, packet: TimedAudioPacket) -> Result<usize> {
        if packet.samples == 0 || packet.sample_rate == 0 || packet.channels == 0 {
            return Ok(0);
        }
        let ch = usize::from(packet.channels);
        let sr = u128::from(packet.sample_rate);
        let packet_frames = u128::try_from(packet.samples / ch).unwrap_or(0);
        if packet_frames == 0 {
            return Ok(0);
        }
        let packet_dur_100ns = u64::try_from((packet_frames * 10_000_000) / sr).unwrap_or(u64::MAX);
        let start_elapsed = packet
            .packet_start_elapsed
            .unwrap_or(packet.elapsed_at_capture);

        if self.first_packet_offset_ms.is_none() {
            self.first_packet_offset_ms =
                Some(u64::try_from(start_elapsed.as_millis()).unwrap_or(u64::MAX));
            self.first_packet_start_elapsed = Some(start_elapsed);
            if packet.qpc_position_100ns > 0 {
                self.stream_origin_qpc_100ns = Some(packet.qpc_position_100ns);
                self.last_packet_end_qpc_100ns =
                    Some(packet.qpc_position_100ns.saturating_add(packet_dur_100ns));
            }
            self.add_samples(packet.samples)?;
            return Ok(0);
        }

        let written_frames = u128::try_from(self.written_samples / ch).unwrap_or(u128::MAX);
        let raw_gap_frames = if let Some(qpc_0) = self
            .stream_origin_qpc_100ns
            .filter(|&q0| packet.qpc_position_100ns >= q0)
        {
            let total_qpc_hns = u128::from(packet.qpc_position_100ns - qpc_0);
            let expected_frames = (total_qpc_hns * sr + 5_000_000) / 10_000_000;
            let cumulative_gap = expected_frames.saturating_sub(written_frames);
            let step_gap_frames = self.last_packet_end_qpc_100ns.map_or(0, |prev_end| {
                let hns = u128::from(packet.qpc_position_100ns.saturating_sub(prev_end));
                (hns * sr + 5_000_000) / 10_000_000
            });
            let step_threshold = (sr / 200).max(1);
            let silent_threshold = (sr / 100).max(1);
            let max_skew = (sr / 50).max(1);
            if (packet.discontinuity && cumulative_gap >= 1)
                || step_gap_frames >= step_threshold
                || (packet.is_silent && cumulative_gap >= silent_threshold)
                || cumulative_gap >= max_skew
            {
                cumulative_gap
            } else {
                0
            }
        } else {
            let first_start = self.first_packet_start_elapsed.unwrap_or(Duration::ZERO);
            let delta = start_elapsed.saturating_sub(first_start);
            let expected_frames = (delta.as_nanos() * sr + 500_000_000) / 1_000_000_000;
            let cumulative_gap = expected_frames.saturating_sub(written_frames);
            let jitter_deadband = (sr / 50).max(1);
            if (packet.discontinuity && cumulative_gap >= 1) || cumulative_gap >= jitter_deadband {
                cumulative_gap
            } else {
                0
            }
        };

        let max_total_frames = (packet.elapsed_at_capture.as_nanos() * sr) / 1_000_000_000;
        let max_gap_frames = max_total_frames.saturating_sub(written_frames + packet_frames);
        let gap_frames = raw_gap_frames.min(max_gap_frames);
        let gap_samples = usize::try_from(gap_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(ch))
            .ok_or_else(|| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "write system audio",
                    "system-audio silence gap sample count overflowed",
                )
            })?;

        if packet.qpc_position_100ns > 0 {
            if self.stream_origin_qpc_100ns.is_none() {
                self.stream_origin_qpc_100ns = Some(packet.qpc_position_100ns);
            }
            self.last_packet_end_qpc_100ns =
                Some(packet.qpc_position_100ns.saturating_add(packet_dur_100ns));
        }
        self.add_samples(gap_samples)?;
        self.add_samples(packet.samples)?;
        Ok(gap_samples)
    }

    fn add_samples(&mut self, samples: usize) -> Result<()> {
        self.written_samples = self.written_samples.checked_add(samples).ok_or_else(|| {
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
#[path = "system_audio_timing_tests.rs"]
mod tests;
