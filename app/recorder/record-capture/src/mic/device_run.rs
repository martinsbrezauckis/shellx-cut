use super::audio_level::RollingAudioLevel;
use super::device::{CapturedMicrophone, MicRecordingGate, MicStreamControl};
use super::stream::run_stream;
use super::wav_layout::MicrophoneWavLayout;
use cpal::traits::DeviceTrait;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use std::time::Instant;

pub(super) struct DeviceCaptureRequest {
    pub(super) path: String,
    pub(super) device: cpal::Device,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) ready: Arc<AtomicBool>,
    pub(super) capture_started: Instant,
    pub(super) meter_peak: Option<Arc<AtomicU32>>,
    pub(super) level: Option<Arc<RollingAudioLevel>>,
    pub(super) recording_gate: Option<Arc<MicRecordingGate>>,
    pub(super) wav_layout: MicrophoneWavLayout,
}

pub(super) fn run_device(request: DeviceCaptureRequest) -> Result<CapturedMicrophone> {
    let DeviceCaptureRequest {
        path,
        device,
        stop,
        ready,
        capture_started,
        meter_peak,
        level,
        recording_gate,
        wav_layout,
    } = request;
    let terminal_level = level.clone();
    let supported = match device.default_input_config() {
        Ok(supported) => supported,
        Err(error) => {
            if let Some(level) = terminal_level {
                level.mark_device_lost();
            }
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "mic config",
                error.to_string(),
            ));
        }
    };
    let end = run_stream(
        &device,
        supported,
        &path,
        None,
        MicStreamControl {
            stop: &stop,
            ready: &ready,
            capture_started,
            meter_peak,
            level,
            recording_gate,
            wav_layout,
        },
    );
    if let Some(level) = terminal_level {
        level.mark_stopped();
    }
    let end = end?;
    Ok(CapturedMicrophone {
        path: end.samples_written.then_some(path),
        microphone_lost: end.microphone_lost,
        first_packet_offset_ms: end.first_packet_offset_ms,
    })
}
