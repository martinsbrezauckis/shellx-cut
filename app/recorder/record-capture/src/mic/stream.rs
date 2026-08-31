use super::device::MicStreamControl;
use super::stream_callback::{capture_origin, mark_first_packet, record_peak};
use super::wav::{
    discard_unpublished_staging, should_publish_microphone, wav_i16_sample_capacity,
    PendingMicSamples,
};
use super::wav_layout::pads_first_packet_offset;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::SampleFormat;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

mod types;
use types::MicStreamEnd;

const MIC_PENDING_SECONDS: u64 = 2;

pub(super) fn run_stream(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    path: &str,
    max_ms: Option<u64>,
    control: MicStreamControl<'_>,
) -> Result<MicStreamEnd> {
    let MicStreamControl {
        stop,
        ready,
        capture_started,
        meter_peak,
        recording_gate,
        wav_layout,
    } = control;
    let fmt = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels;
    let sample_rate = config.sample_rate.0;
    let max_pending_samples = u64::from(sample_rate)
        .checked_mul(u64::from(channels))
        .and_then(|samples| samples.checked_mul(MIC_PENDING_SECONDS))
        .and_then(|samples| usize::try_from(samples).ok())
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "microphone format",
                "microphone callback buffer size overflowed",
            )
        })?;

    let pending_samples = Arc::new(Mutex::new(PendingMicSamples::default()));
    let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let first_packet_offset_ms = Arc::new(AtomicU64::new(u64::MAX));
    let err_fn = {
        let stream_error = stream_error.clone();
        move |error: cpal::StreamError| {
            eprintln!("audio stream error: {error}");
            let mut slot = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(error.to_string());
            }
        }
    };

    let b = pending_samples.clone();
    let rdy = ready.clone();
    let first = first_packet_offset_ms.clone();
    let meter_f32 = meter_peak.clone();
    let meter_i16 = meter_peak.clone();
    let meter_u16 = meter_peak;
    let gate_f32 = recording_gate.clone();
    let gate_i16 = recording_gate.clone();
    let gate_u16 = recording_gate;
    let stream = match fmt {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |d: &[f32], _: &cpal::InputCallbackInfo| {
                let Some(origin) = capture_origin(&rdy, gate_f32.as_ref(), capture_started) else {
                    return;
                };
                mark_first_packet(&rdy, &first, origin);
                if let Some(peak) = meter_f32.as_deref() {
                    for &sample in d {
                        record_peak(peak, (sample.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                }
                let mut g = b.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.extend(
                    d.iter().map(|&v| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
                    d.len(),
                    max_pending_samples,
                );
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |d: &[i16], _: &cpal::InputCallbackInfo| {
                let Some(origin) = capture_origin(&rdy, gate_i16.as_ref(), capture_started) else {
                    return;
                };
                mark_first_packet(&rdy, &first, origin);
                if let Some(peak) = meter_i16.as_deref() {
                    for &sample in d {
                        record_peak(peak, sample);
                    }
                }
                b.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(d.iter().copied(), d.len(), max_pending_samples);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |d: &[u16], _: &cpal::InputCallbackInfo| {
                let Some(origin) = capture_origin(&rdy, gate_u16.as_ref(), capture_started) else {
                    return;
                };
                mark_first_packet(&rdy, &first, origin);
                if let Some(peak) = meter_u16.as_deref() {
                    for &sample in d {
                        record_peak(peak, (i32::from(sample) - 32768) as i16);
                    }
                }
                let mut g = b.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.extend(
                    d.iter().map(|&v| (v as i32 - 32768) as i16),
                    d.len(),
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
            ))
        }
    }
    .map_err(|e| RecordError::new(error_codes::CAPTURE, "build audio stream", e.to_string()))?;
    let sample_capacity = wav_i16_sample_capacity(channels)?;
    let output_path = std::path::Path::new(path);
    let parent = output_path.parent().ok_or_else(|| {
        RecordError::new(
            error_codes::IO,
            "reserve microphone audio staging",
            "microphone audio path has no parent",
        )
    })?;
    let (temp_path, staging) =
        record_recovery::create_staging_file(parent, "mic-wav").map_err(|error| {
            RecordError::new(
                error_codes::IO,
                "reserve microphone audio staging",
                error.to_string(),
            )
        })?;
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = match hound::WavWriter::new(staging, spec) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(RecordError::new(
                error_codes::IO,
                "create microphone audio",
                error.to_string(),
            ));
        }
    };
    if let Err(error) = stream.play() {
        drop(writer);
        let _ = std::fs::remove_file(&temp_path);
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "start microphone audio",
            error.to_string(),
        ));
    }

    let (capture_error, microphone_lost) = {
        let mut written_samples = 0_u64;
        let mut leading_silence_written = false;
        let mut drain_buffer = Vec::new();
        let mut drain_pending = || -> Result<bool> {
            let overflowed = {
                let mut pending = pending_samples
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::swap(&mut drain_buffer, &mut pending.samples);
                std::mem::take(&mut pending.overflowed)
            };
            if overflowed {
                return Err(RecordError::new(
                    error_codes::CAPTURE,
                    "capture microphone audio",
                    "microphone audio could not be written to disk fast enough",
                ));
            }
            if !leading_silence_written && !drain_buffer.is_empty() {
                let first_offset_ms = first_packet_offset_ms.load(Ordering::Relaxed);
                if pads_first_packet_offset(wav_layout, first_offset_ms) {
                    let wanted = crate::mic_timing::leading_silence_samples(
                        first_offset_ms,
                        sample_rate,
                        channels,
                    );
                    let silence = wanted.min(sample_capacity.saturating_sub(written_samples));
                    for _ in 0..silence {
                        writer.write_sample(0_i16).map_err(|e| {
                            RecordError::new(
                                error_codes::IO,
                                "pad microphone audio to capture clock",
                                e.to_string(),
                            )
                        })?;
                    }
                    written_samples = written_samples.saturating_add(silence);
                }
                leading_silence_written = true;
            }
            let remaining = sample_capacity.saturating_sub(written_samples);
            let write_count = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(drain_buffer.len());
            for sample in drain_buffer.drain(..write_count) {
                writer.write_sample(sample).map_err(|e| {
                    RecordError::new(error_codes::IO, "write microphone audio", e.to_string())
                })?;
            }
            let write_count = u64::try_from(write_count).map_err(|_| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "write microphone audio",
                    "microphone sample count overflowed",
                )
            })?;
            written_samples += write_count;
            let reached_capacity = !drain_buffer.is_empty() || written_samples >= sample_capacity;
            drain_buffer.clear();
            Ok(reached_capacity)
        };

        let started = std::time::Instant::now();
        let mut capture_error = None;
        let mut microphone_lost = false;
        while !stop.load(Ordering::Relaxed) {
            if let Some(ms) = max_ms {
                if started.elapsed() >= Duration::from_millis(ms) {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(40));
            if let Some(error) = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                eprintln!("microphone stream ended: {error}");
                microphone_lost = true;
                break;
            }
            match drain_pending() {
                Ok(true) => {
                    // Classic WAV cannot represent another full frame. End the shared capture so
                    // video and audio stay aligned instead of silently truncating or corrupting audio.
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    capture_error = Some(error);
                    break;
                }
            }
        }
        drop(stream); // stop capture

        if capture_error.is_none() {
            if microphone_lost {
                if let Err(error) = drain_pending() {
                    capture_error = Some(error);
                }
            } else if let Some(error) = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                capture_error = Some(RecordError::new(
                    error_codes::CAPTURE,
                    "capture microphone audio",
                    error,
                ));
            } else if let Err(error) = drain_pending() {
                capture_error = Some(error);
            }
        }
        (capture_error, microphone_lost)
    };
    if let Some(error) = capture_error {
        drop(writer);
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    let samples_written = writer.len() > 0;
    if !should_publish_microphone(samples_written) {
        let _ = writer.finalize();
        discard_unpublished_staging(&temp_path);
        return Ok(MicStreamEnd {
            microphone_lost,
            samples_written: false,
            first_packet_offset_ms: None,
        });
    }
    if let Err(error) = writer.finalize() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(RecordError::new(
            error_codes::IO,
            "finalize microphone audio",
            error.to_string(),
        ));
    }
    if let Err(error) = record_recovery::publish_new_synced(&temp_path, output_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(RecordError::new(
            error_codes::IO,
            "publish microphone audio",
            error.to_string(),
        ));
    }
    Ok(MicStreamEnd {
        microphone_lost,
        samples_written,
        first_packet_offset_ms: (first_packet_offset_ms.load(Ordering::Relaxed) != u64::MAX)
            .then(|| first_packet_offset_ms.load(Ordering::Relaxed)),
    })
}
