use super::audio_meters::CaptureAudioMeters;

#[test]
fn admitted_meter_is_live_only_for_current_native_packets() {
    let meters = CaptureAudioMeters::new(true, false);
    let awaiting = meters.status();
    assert_eq!(awaiting.microphone.state, "awaiting_samples");
    assert!(awaiting.microphone.stale);

    let microphone = meters
        .microphone_meter()
        .expect("admitted microphone meter");
    microphone.observe_i16(&[i16::MAX]);
    let live = meters.status();
    assert_eq!(live.microphone.state, "live");
    assert!(!live.microphone.stale);
    assert!(live.microphone.clipping);

    microphone.mark_device_lost();
    let lost = meters.status();
    assert_eq!(lost.microphone.state, "device_lost");
    assert!(lost.microphone.stale);
    assert!(!lost.microphone.clipping);
}

#[test]
fn terminalization_closes_native_handles_without_erasing_device_loss() {
    let meters = CaptureAudioMeters::new(true, true);
    meters.mark_terminal();
    let status = meters.status();
    assert_eq!(status.microphone.state, "stopped");
    assert_eq!(
        status.system_audio.state,
        if cfg!(target_os = "macos") {
            "unavailable"
        } else {
            "stopped"
        }
    );
    assert!(status.microphone.stale);
    assert!(status.system_audio.stale);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_system_meter_names_the_active_stream_limit_without_faking_samples() {
    let status = CaptureAudioMeters::new(false, true).status().system_audio;
    assert_eq!(status.state, "unavailable");
    assert!(status.stale);
    assert!(!status.clipping);
    assert!(status.peak_dbfs.is_none());
    assert!(status
        .detail
        .is_some_and(|detail| detail.contains("SCRecordingOutput")));
    assert!(status
        .detail
        .is_some_and(|detail| detail.contains("only at Stop")));
}
