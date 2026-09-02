use super::{packet_samples, record_packet, State};
use crate::system_audio_timing::SystemAudioTimingTracker;
use std::fs::File;
use std::io::BufWriter;
use std::time::Duration;

fn state() -> State {
    let path = tempfile::NamedTempFile::new().unwrap();
    let writer = hound::WavWriter::new(
        BufWriter::new(File::create(path.path()).unwrap()),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    State {
        format: Default::default(),
        timing: SystemAudioTimingTracker::default(),
        level: None,
        writer: Some(writer),
        error: None,
    }
}

#[test]
fn first_nonempty_pipewire_packet_keeps_a_delayed_capture_clock_offset() {
    let mut state = state();
    assert_eq!(packet_samples(&[]), None, "empty buffers are not packets");
    record_packet(&mut state, &[0, 0, 1, 0], Duration::from_millis(2_350)).unwrap();
    assert_eq!(state.timing.first_packet_offset_ms(), Some(2_350));
}

#[test]
fn malformed_packet_cannot_create_timing_or_wav_samples() {
    let mut state = state();
    assert!(record_packet(&mut state, &[0], Duration::ZERO).is_err());
    assert_eq!(state.timing.first_packet_offset_ms(), None);
}

#[test]
fn valid_pipewire_packet_feeds_the_caller_owned_level() {
    let level = std::sync::Arc::new(crate::mic::RollingAudioLevel::new());
    let mut state = state();
    state.level = Some(level.clone());

    record_packet(&mut state, &[0, 0, 0, 128], Duration::ZERO).unwrap();

    assert_eq!(state.timing.first_packet_offset_ms(), Some(0));
    assert_eq!(level.snapshot().peak_dbfs, Some(0.0));
}
