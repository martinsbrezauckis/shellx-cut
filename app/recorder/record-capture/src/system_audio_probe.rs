//! Short, user-triggered native system-audio delivery probe.

use record_core::{error_codes, RecordError, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
use std::time::Duration;

pub const DEFAULT_WINDOW_MS: u64 = 2_500;
const MIN_WINDOW_MS: u64 = 500;
const MAX_WINDOW_MS: u64 = 5_000;
const MIN_SIGNAL_AMPLITUDE: f32 = 0.001;
static SYSTEM_AUDIO_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Process-local ownership for the one native system-audio stream. A probe and
/// a recording must never compete for the platform tap/loopback device.
pub struct SystemAudioLease(());

pub fn reserve_system_audio() -> Result<SystemAudioLease> {
    SYSTEM_AUDIO_ACTIVE
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map(|_| SystemAudioLease(()))
        .map_err(|_| {
            RecordError::new(
                error_codes::CAPTURE,
                "system audio is already in use",
                "another recording or system-audio test owns the native audio stream",
            )
            .with_action("wait for the current recording or audio test to finish, then retry")
        })
}

impl Drop for SystemAudioLease {
    fn drop(&mut self) {
        SYSTEM_AUDIO_ACTIVE.store(false, Ordering::Release);
    }
}

/// Packet facts from one bounded native system-audio probe. No captured audio or
/// temporary path leaves this API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemAudioProbe {
    pub supported: bool,
    pub live: bool,
    pub backend: String,
    pub window_ms: u64,
    pub first_packet_offset_ms: Option<u64>,
    pub sample_frames: u64,
    pub signal_detected: bool,
    pub detail: String,
}

pub fn probe_system_audio(requested_ms: u64) -> Result<SystemAudioProbe> {
    let _lease = reserve_system_audio()?;
    let window_ms = bounded_window(requested_ms);

    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return probe_wav(window_ms, "PipeWire", |path, stop, started| {
            crate::linux_system_audio::capture_system_pipewire(path, Some(window_ms), stop, started)
        });
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return probe_wav(
            window_ms,
            "Windows process loopback",
            |path, stop, started| {
                crate::mic::capture_system_loopback(
                    &path.to_string_lossy(),
                    Some(window_ms),
                    stop,
                    started,
                )
            },
        );
    }

    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        let tap = crate::macos_system_tap::SystemAudioTap::start(0).ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "start system audio test",
                "macOS did not open the Core Audio process tap",
            )
            .with_action(
                "Allow Audio Capture for ShellX Cut in System Settings, restart Cut if macOS asks, then test again",
            )
        })?;
        std::thread::sleep(Duration::from_millis(window_ms));
        let result = tap.finish();
        if result.rc != 0 {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "finish system audio test",
                format!("Core Audio process tap stopped with status {}", result.rc),
            ));
        }
        let sample_frames = if result.samples.is_some() {
            result
                .count
                .checked_div(u64::from(result.channels))
                .unwrap_or(0)
        } else {
            0
        };
        let signal_detected = result
            .samples
            .as_ref()
            .is_some_and(|samples| samples.as_slice().iter().copied().any(has_signal));
        return Ok(probe_result(
            "Core Audio process tap",
            window_ms,
            result.first_packet_offset_ms,
            sample_frames,
            signal_detected,
        ));
    }

    #[allow(unreachable_code)]
    Ok(SystemAudioProbe {
        supported: false,
        live: false,
        backend: "unavailable".into(),
        window_ms,
        first_packet_offset_ms: None,
        sample_frames: 0,
        signal_detected: false,
        detail: "This build has no native system-audio capture backend.".into(),
    })
}

#[cfg(any(
    all(target_os = "linux", feature = "capture-linux"),
    all(windows, feature = "capture-windows")
))]
fn probe_wav(
    window_ms: u64,
    backend: &str,
    capture: impl FnOnce(
        &std::path::Path,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::time::Instant,
    ) -> Result<crate::SystemAudioCapture>,
) -> Result<SystemAudioProbe> {
    let stage = record_recovery::PrivateStaging::create(
        &std::env::temp_dir(),
        "system-audio-probe",
        "system.wav",
    )
    .map_err(|error| {
        RecordError::new(
            error_codes::IO,
            "prepare system audio test",
            error.to_string(),
        )
    })?;
    let result = (|| {
        let captured = capture(
            stage.path(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            std::time::Instant::now(),
        )?;
        let mut reader = hound::WavReader::open(stage.path()).map_err(|error| {
            RecordError::new(
                error_codes::IO,
                "inspect system audio test",
                error.to_string(),
            )
        })?;
        let sample_frames = u64::from(reader.duration());
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
            return Err(RecordError::new(
                error_codes::IO,
                "inspect system audio test",
                format!(
                    "expected 16-bit PCM, got {:?}/{}-bit",
                    spec.sample_format, spec.bits_per_sample
                ),
            ));
        }
        let mut signal_detected = false;
        for sample in reader.samples::<i16>() {
            let sample = sample.map_err(|error| {
                RecordError::new(
                    error_codes::IO,
                    "inspect system audio test samples",
                    error.to_string(),
                )
            })?;
            signal_detected |= has_signal(f32::from(sample) / f32::from(i16::MAX));
        }
        Ok(probe_result(
            backend,
            window_ms,
            captured.first_packet_offset_ms,
            sample_frames,
            signal_detected,
        ))
    })();
    let cleanup = stage.cleanup().map_err(|error| {
        RecordError::new(
            error_codes::IO,
            "remove system audio test data",
            error.to_string(),
        )
    });
    cleanup.and(result)
}

fn bounded_window(requested_ms: u64) -> u64 {
    requested_ms.clamp(MIN_WINDOW_MS, MAX_WINDOW_MS)
}

fn probe_result(
    backend: &str,
    window_ms: u64,
    first_packet_offset_ms: Option<u64>,
    sample_frames: u64,
    signal_detected: bool,
) -> SystemAudioProbe {
    let live = first_packet_offset_ms.is_some() && sample_frames > 0;
    let detail = if live && signal_detected {
        format!("{backend} delivered {sample_frames} audio frames with a detected signal.")
    } else if live {
        format!(
            "{backend} delivered audio frames, but they were silent. Play a sound and check the output route, then test again."
        )
    } else {
        format!("{backend} opened, but no audio packet arrived. Play a sound and test again.")
    };
    SystemAudioProbe {
        supported: true,
        live,
        backend: backend.into(),
        window_ms,
        first_packet_offset_ms,
        sample_frames,
        signal_detected,
        detail,
    }
}

fn has_signal(sample: f32) -> bool {
    sample.is_finite() && sample.abs() >= MIN_SIGNAL_AMPLITUDE
}

#[cfg(test)]
mod tests {
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
}
