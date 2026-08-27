use super::{bounded_window, has_signal, probe_result, reserve_system_audio};

#[test]
fn probe_window_is_short_and_bounded() {
    assert_eq!(bounded_window(0), 500);
    assert_eq!(bounded_window(2_500), 2_500);
    assert_eq!(bounded_window(u64::MAX), 5_000);
}

#[test]
fn packet_facts_never_infer_delivery_from_elapsed_time() {
    let none = probe_result("fixture", 2_500, None, 96_000, true);
    assert!(!none.live);
    let empty = probe_result("fixture", 2_500, Some(20), 0, true);
    assert!(!empty.live);
    let live = probe_result("fixture", 2_500, Some(20), 96_000, true);
    assert!(live.live);
    assert!(live.signal_detected);
    assert_eq!(live.first_packet_offset_ms, Some(20));
}

#[test]
fn delivered_silence_is_not_reported_as_detected_signal() {
    let silent = probe_result("fixture", 2_500, Some(20), 96_000, false);
    assert!(silent.live, "packet delivery remains a separate fact");
    assert!(!silent.signal_detected);
    assert!(silent.detail.contains("were silent"));
}

#[test]
fn signal_threshold_ignores_digital_silence_and_non_finite_samples() {
    assert!(!has_signal(0.0009));
    assert!(has_signal(0.001));
    assert!(has_signal(-0.25));
    assert!(!has_signal(f32::NAN));
}

#[test]
fn native_stream_lease_prevents_probe_recording_overlap() {
    let first = reserve_system_audio().unwrap();
    assert!(reserve_system_audio().is_err());
    drop(first);
    assert!(reserve_system_audio().is_ok());
}

#[cfg(any(
    all(target_os = "linux", feature = "capture-linux"),
    all(windows, feature = "capture-windows")
))]
#[test]
fn successful_probe_removes_its_private_wav_and_directory() {
    use std::sync::{Arc, Mutex};

    let observed = Arc::new(Mutex::new(None));
    let path_out = observed.clone();
    let probe = super::probe_wav(500, "fixture", move |path, _stop, _started| {
        *path_out.lock().unwrap() = Some(path.to_path_buf());
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample(100_i16).unwrap();
        writer.write_sample(100_i16).unwrap();
        writer.finalize().unwrap();
        Ok(crate::SystemAudioCapture {
            path: path.to_string_lossy().into_owned(),
            first_packet_offset_ms: Some(7),
        })
    })
    .unwrap();

    assert!(probe.live);
    assert!(probe.signal_detected);
    let path = observed.lock().unwrap().clone().unwrap();
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}
