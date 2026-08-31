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
        recording_gate,
        wav_layout,
    } = request;
    let supported = device
        .default_input_config()
        .map_err(|e| RecordError::new(error_codes::CAPTURE, "mic config", e.to_string()))?;
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
            recording_gate,
            wav_layout,
        },
    )?;
    Ok(CapturedMicrophone {
        path: end.samples_written.then_some(path),
        microphone_lost: end.microphone_lost,
        first_packet_offset_ms: end.first_packet_offset_ms,
    })
}
