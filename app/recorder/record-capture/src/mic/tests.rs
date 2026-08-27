use super::device::{join_bounded, peak_dbfs};
use super::loopback_pcm::decode_process_loopback_packet;
use super::wav::{
    discard_unpublished_staging, should_publish_microphone, wav_i16_sample_capacity,
    PendingMicSamples, WAV_HEADER_MARGIN_BYTES,
};
use record_core::Result;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn decodes_fixed_process_loopback_pcm() {
    let bytes = [i16::MIN, -1, 0, i16::MAX]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(
        decode_process_loopback_packet(Some(&bytes), 2, false).unwrap(),
        vec![i16::MIN, -1, 0, i16::MAX]
    );
}

#[test]
fn rejects_truncated_process_loopback_packet() {
    assert!(decode_process_loopback_packet(Some(&[0; 7]), 2, false).is_err());
}

#[test]
fn silent_packet_does_not_dereference_audio_data() {
    assert_eq!(
        decode_process_loopback_packet(None, 2, true).unwrap(),
        vec![0; 4]
    );
}

#[test]
fn classic_wav_capacity_is_frame_aligned() {
    let stereo_capacity = wav_i16_sample_capacity(2).unwrap();
    assert_eq!(stereo_capacity % 2, 0);
    assert!(stereo_capacity * 2 + WAV_HEADER_MARGIN_BYTES <= u64::from(u32::MAX));
    assert!(wav_i16_sample_capacity(0).is_err());
}

#[test]
fn microphone_callback_backlog_is_bounded() {
    let mut pending = PendingMicSamples::default();
    pending.extend([1_i16, 2, 3, 4], 4, 3);
    assert_eq!(pending.samples, [1, 2, 3]);
    assert!(pending.overflowed);
}

#[test]
fn zero_sample_microphone_stream_never_publishes_a_track() {
    assert!(!should_publish_microphone(false));
    assert!(should_publish_microphone(true));
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("mic-wav.tmp");
    std::fs::write(&staging, b"empty wav staging").unwrap();
    discard_unpublished_staging(&staging);
    assert!(!staging.exists());
}

#[test]
fn microphone_peak_is_measured_without_a_fake_silence_floor() {
    assert_eq!(peak_dbfs(0), None);
    assert_eq!(peak_dbfs(u32::from(i16::MAX as u16)), Some(0));
    assert_eq!(peak_dbfs(3_277), Some(-20));
}

#[test]
fn microphone_join_does_not_wait_for_a_stuck_worker() {
    // The property under test is "join gives up instead of waiting for the
    // worker", so what matters is the SEPARATION between the join timeout
    // and the worker's lifetime — not a tight wall-clock number.
    //
    // This previously slept 150 ms, timed out at 20 ms and asserted under
    // 120 ms, leaving only 30 ms between "gave up early" and "waited for the
    // worker". That is inside normal scheduler noise on shared CI hardware,
    // and it failed on a GitHub macOS runner while passing everywhere else.
    // Hold the worker on a channel until the assertion completes. This is a
    // deterministic stuck-worker condition without leaving a detached
    // three-second sleeper behind after the test.
    let (release, hold) = mpsc::channel::<()>();
    let handle = thread::spawn(move || -> Result<String> {
        let _ = hold.recv();
        Ok("late.wav".into())
    });
    let started = std::time::Instant::now();
    assert!(join_bounded(handle, Duration::from_millis(20)).is_none());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "join_bounded returned after {:?}; it must abandon the blocked worker, not wait for it",
        started.elapsed()
    );
    drop(release);
}

#[test]
fn microphone_join_keeps_a_completed_result() {
    let handle = thread::spawn(|| -> Result<String> { Ok("mic.wav".into()) });
    assert_eq!(
        join_bounded(handle, Duration::from_secs(1))
            .expect("worker should join")
            .expect("worker should succeed"),
        "mic.wav"
    );
}
