//! Microphone packet timing projected onto the shared capture clock.

/// Convert a first-packet offset into interleaved i16 silence samples. Saturation
/// keeps a bad native timestamp from overflowing a WAV writer calculation.
pub(crate) fn leading_silence_samples(offset_ms: u64, sample_rate: u32, channels: u16) -> u64 {
    offset_ms
        .saturating_mul(u64::from(sample_rate))
        .saturating_div(1_000)
        .saturating_mul(u64::from(channels))
}

#[cfg(test)]
mod tests {
    use super::leading_silence_samples;

    #[test]
    fn first_packet_offset_becomes_interleaved_silence() {
        assert_eq!(leading_silence_samples(0, 48_000, 2), 0);
        assert_eq!(leading_silence_samples(37, 48_000, 2), 3_552);
        assert_eq!(leading_silence_samples(1_000, 44_100, 1), 44_100);
    }
}
