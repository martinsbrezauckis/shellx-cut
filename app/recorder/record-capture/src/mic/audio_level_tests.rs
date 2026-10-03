use super::audio_level::{AudioLevelLifecycle, RollingAudioLevel, ROLLING_WINDOW, STALE_AFTER};
use std::time::{Duration, Instant};

#[test]
fn measures_real_peak_rms_and_decay_from_one_callback() {
    let level = RollingAudioLevel::new();
    let start = Instant::now();
    level.observe_i16_at(&[0, 16_384, i16::MIN], start);

    let snapshot = level.snapshot_at(start);
    assert_eq!(snapshot.peak_dbfs, Some(0.0));
    assert!((snapshot.rms_dbfs.expect("RMS") + 3.8).abs() < 0.1);
    assert_eq!(snapshot.decayed_peak_dbfs, Some(0.0));
    assert_eq!(snapshot.sample_age_ms, Some(0));
    assert!(!snapshot.stale);
    assert!(snapshot.clipping);
    assert_eq!(snapshot.lifecycle, AudioLevelLifecycle::Active);

    let decayed = level.snapshot_at(start + Duration::from_millis(700));
    assert!(decayed.decayed_peak_dbfs.expect("decayed peak") < -10.0);
    assert!(!decayed.stale);
}

#[test]
fn terminal_or_lost_streams_are_stale_and_cannot_accept_new_packets() {
    let level = RollingAudioLevel::new();
    let start = Instant::now();
    level.observe_i16_at(&[16_384], start);
    level.mark_device_lost();
    level.observe_i16_at(&[i16::MAX], start + Duration::from_millis(1));

    let lost = level.snapshot_at(start + Duration::from_millis(2));
    assert_eq!(lost.lifecycle, AudioLevelLifecycle::DeviceLost);
    assert!(lost.stale);
    assert!(!lost.clipping, "post-loss packet must not be admitted");

    level.mark_stopped();
    assert_eq!(
        level
            .snapshot_at(start + Duration::from_millis(3))
            .lifecycle,
        AudioLevelLifecycle::DeviceLost,
        "ordinary cancellation must retain a preceding device-loss fact"
    );
}

#[test]
fn expires_rolling_values_and_marks_an_idle_callback_stale() {
    let level = RollingAudioLevel::new();
    let start = Instant::now();
    level.observe_i16_at(&[i16::MAX], start);

    let expired = level.snapshot_at(start + ROLLING_WINDOW + Duration::from_millis(1));
    assert_eq!(expired.peak_dbfs, None);
    assert_eq!(expired.rms_dbfs, None);
    assert!(!expired.stale);

    let stale = level.snapshot_at(start + STALE_AFTER + Duration::from_millis(1));
    assert!(stale.stale);
    assert_eq!(stale.sample_age_ms, Some(1_201));
}

#[test]
fn accepts_each_native_pcm_shape_without_a_synthetic_signal() {
    let start = Instant::now();

    let f32_level = RollingAudioLevel::new();
    f32_level.observe_f32_at(&[f32::NAN, f32::INFINITY, -0.5], start);
    assert!((f32_level.snapshot_at(start).peak_dbfs.expect("peak") + 6.0).abs() < 0.1);

    let u16_level = RollingAudioLevel::new();
    u16_level.observe_u16_at(&[0, 32_768, u16::MAX], start);
    assert_eq!(u16_level.snapshot_at(start).peak_dbfs, Some(0.0));

    let bytes_level = RollingAudioLevel::new();
    bytes_level.observe_s16le_bytes_at(&[0, 0, 0, 128, 0], start);
    assert_eq!(bytes_level.snapshot_at(start).peak_dbfs, Some(0.0));
}

#[test]
fn joined_pause_segment_clears_samples_and_resume_accepts_only_new_real_packets() {
    let level = RollingAudioLevel::new();
    let start = Instant::now();
    level.observe_i16_at(&[i16::MIN], start);
    assert!(level.snapshot_at(start).clipping);
    level.finish_segment(false);
    let paused = level.snapshot_at(start);
    assert_eq!(paused.lifecycle, AudioLevelLifecycle::Active);
    assert_eq!(paused.peak_dbfs, None);
    assert_eq!(paused.sample_age_ms, None);
    assert!(paused.stale);
    level.observe_i16_at(&[16_384], start + Duration::from_millis(1));
    let resumed = level.snapshot_at(start + Duration::from_millis(1));
    assert!(!resumed.stale);
    assert!(!resumed.clipping);
    assert!((resumed.peak_dbfs.unwrap() + 6.0).abs() < 0.1);
    level.mark_stopped();
    level.finish_segment(false);
    level.observe_i16_at(&[i16::MIN], start + Duration::from_millis(2));
    let stopped = level.snapshot_at(start + Duration::from_millis(2));
    assert_eq!(stopped.lifecycle, AudioLevelLifecycle::Stopped);
    assert!(stopped.stale);
    assert!(!stopped.clipping);
}

#[test]
fn failed_pause_segment_is_terminal_and_successful_close_cannot_revive_it() {
    let level = RollingAudioLevel::new();
    let start = Instant::now();
    level.observe_i16_at(&[16_384], start);
    level.finish_segment(true);
    level.finish_segment(false);
    level.mark_stopped();
    level.observe_i16_at(&[i16::MIN], start + Duration::from_millis(1));
    let failed = level.snapshot_at(start + Duration::from_millis(1));
    assert_eq!(failed.lifecycle, AudioLevelLifecycle::DeviceLost);
    assert!(failed.stale);
    assert!(!failed.clipping);
}
