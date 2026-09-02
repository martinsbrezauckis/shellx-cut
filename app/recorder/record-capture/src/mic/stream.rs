use super::device::MicStreamControl;
use super::stream_callbacks::{build_input_stream, MicCallbackControl, MicInputStream};
use super::wav::{discard_unpublished_staging, should_publish_microphone, wav_i16_sample_capacity};
use super::wav_layout::pads_first_packet_offset;
use cpal::traits::StreamTrait;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::Ordering;
use std::sync::Arc;
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
        level,
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

    let MicInputStream {
        stream,
        pending_samples,
        stream_error,
        first_packet_offset_ms,
    } = build_input_stream(
        device,
        &config,
        fmt,
        max_pending_samples,
        MicCallbackControl {
            ready: Arc::clone(ready),
            capture_started,
            meter_peak,
            level,
            recording_gate,
        },
    )?;
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
