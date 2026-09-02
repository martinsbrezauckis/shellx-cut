//! CPAL callback construction for microphone capture and live meter updates.

use super::audio_level::RollingAudioLevel;
use super::device::MicRecordingGate;
use super::stream_callback::{capture_origin, mark_first_packet, record_peak};
use super::wav::PendingMicSamples;
use cpal::traits::DeviceTrait;
use cpal::SampleFormat;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) struct MicCallbackControl {
    pub(super) ready: Arc<AtomicBool>,
    pub(super) capture_started: Instant,
    pub(super) meter_peak: Option<Arc<AtomicU32>>,
    pub(super) level: Option<Arc<RollingAudioLevel>>,
    pub(super) recording_gate: Option<Arc<MicRecordingGate>>,
}

pub(super) struct MicInputStream {
    pub(super) stream: cpal::Stream,
    pub(super) pending_samples: Arc<Mutex<PendingMicSamples>>,
    pub(super) stream_error: Arc<Mutex<Option<String>>>,
    pub(super) first_packet_offset_ms: Arc<AtomicU64>,
}

pub(super) fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: SampleFormat,
    max_pending_samples: usize,
    control: MicCallbackControl,
) -> Result<MicInputStream> {
    let MicCallbackControl {
        ready,
        capture_started,
        meter_peak,
        level,
        recording_gate,
    } = control;
    let pending_samples = Arc::new(Mutex::new(PendingMicSamples::default()));
    let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let first_packet_offset_ms = Arc::new(AtomicU64::new(u64::MAX));
    let error_level = level.clone();
    let err_fn = {
        let stream_error = stream_error.clone();
        move |error: cpal::StreamError| {
            eprintln!("audio stream error: {error}");
            if let Some(level) = error_level.as_deref() {
                level.mark_device_lost();
            }
            let mut slot = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(error.to_string());
            }
        }
    };

    let pending = pending_samples.clone();
    let ready_for_callback = ready.clone();
    let first_packet = first_packet_offset_ms.clone();
    let meter_f32 = meter_peak.clone();
    let meter_i16 = meter_peak.clone();
    let meter_u16 = meter_peak;
    let level_f32 = level.clone();
    let level_i16 = level.clone();
    let level_u16 = level;
    let gate_f32 = recording_gate.clone();
    let gate_i16 = recording_gate.clone();
    let gate_u16 = recording_gate;
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                let Some(origin) =
                    capture_origin(&ready_for_callback, gate_f32.as_ref(), capture_started)
                else {
                    return;
                };
                mark_first_packet(&ready_for_callback, &first_packet, origin);
                if let Some(peak) = meter_f32.as_deref() {
                    for &sample in samples {
                        record_peak(peak, (sample.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                }
                if let Some(level) = level_f32.as_deref() {
                    level.observe_f32(samples);
                }
                let mut pending = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.extend(
                    samples
                        .iter()
                        .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16),
                    samples.len(),
                    max_pending_samples,
                );
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config,
            move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                let Some(origin) =
                    capture_origin(&ready_for_callback, gate_i16.as_ref(), capture_started)
                else {
                    return;
                };
                mark_first_packet(&ready_for_callback, &first_packet, origin);
                if let Some(peak) = meter_i16.as_deref() {
                    for &sample in samples {
                        record_peak(peak, sample);
                    }
                }
                if let Some(level) = level_i16.as_deref() {
                    level.observe_i16(samples);
                }
                pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(samples.iter().copied(), samples.len(), max_pending_samples);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            config,
            move |samples: &[u16], _: &cpal::InputCallbackInfo| {
                let Some(origin) =
                    capture_origin(&ready_for_callback, gate_u16.as_ref(), capture_started)
                else {
                    return;
                };
                mark_first_packet(&ready_for_callback, &first_packet, origin);
                if let Some(peak) = meter_u16.as_deref() {
                    for &sample in samples {
                        record_peak(peak, (i32::from(sample) - 32768) as i16);
                    }
                }
                if let Some(level) = level_u16.as_deref() {
                    level.observe_u16(samples);
                }
                let mut pending = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.extend(
                    samples
                        .iter()
                        .map(|&sample| (i32::from(sample) - 32768) as i16),
                    samples.len(),
                    max_pending_samples,
                );
            },
            err_fn,
            None,
        ),
        other => {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "audio format",
                format!("unsupported sample format {other:?}"),
            ));
        }
    }
    .map_err(|error| {
        RecordError::new(
            error_codes::CAPTURE,
            "build audio stream",
            error.to_string(),
        )
    })?;

    Ok(MicInputStream {
        stream,
        pending_samples,
        stream_error,
        first_packet_offset_ms,
    })
}
