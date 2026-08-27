use super::activation::{
    trace_windows_loopback, windows_error, ComApartment, ProcessLoopbackActivation, WindowsEvent,
    PROCESS_LOOPBACK_BITS_PER_SAMPLE, PROCESS_LOOPBACK_BLOCK_ALIGN, PROCESS_LOOPBACK_CHANNELS,
    PROCESS_LOOPBACK_SAMPLE_RATE,
};
use crate::system_audio_timing::{SystemAudioCapture, SystemAudioTimingTracker};
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler;

/// Capture desktop audio through Windows' endpoint-independent process-loopback device.
/// Excluding Cut's daemon process tree yields the rest of the system mix without opening a
/// physical render driver. Requires Windows build 20348+; older builds fail cleanly.
pub fn capture_system_loopback(
    path: &str,
    max_ms: Option<u64>,
    stop: Arc<AtomicBool>,
    capture_started: Instant,
) -> Result<SystemAudioCapture> {
    capture_process_loopback(path, max_ms, stop, capture_started)
}

/// Process-loopback implementation. Writes a fixed 48 kHz stereo 16-bit WAV.
fn capture_process_loopback(
    path: &str,
    max_ms: Option<u64>,
    stop: Arc<AtomicBool>,
    capture_started: Instant,
) -> Result<SystemAudioCapture> {
    use std::mem::{size_of, ManuallyDrop};
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        ActivateAudioInterfaceAsync, IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT,
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
        AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Com::BLOB;
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::System::Variant::VT_BLOB;

    // Classic WAV uses 32-bit chunk lengths. Six hours of 48 kHz stereo i16 is
    // 4,147,200,000 data bytes, leaving enough header/packet margin below 4 GiB.
    const LOOPBACK_CEILING_MS: u64 = 6 * 60 * 60 * 1000;
    let limit =
        Duration::from_millis(max_ms.map_or(LOOPBACK_CEILING_MS, |ms| ms.min(LOOPBACK_CEILING_MS)));
    let _apartment = ComApartment::initialize()?;
    trace_windows_loopback("process-loopback COM initialized");

    let mut activation = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: std::process::id(),
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let activation_blob = BLOB {
        cbSize: u32::try_from(size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()).unwrap(),
        pBlobData: (&mut activation as *mut AUDIOCLIENT_ACTIVATION_PARAMS).cast(),
    };
    // This VT_BLOB borrows `activation`; PROPVARIANT's generated Drop calls
    // PropVariantClear and would otherwise try to free that stack-owned pointer.
    let activation_variant = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: activation_blob,
                },
            }),
        },
    });
    let event = Arc::new(WindowsEvent::new()?);
    let activation_result = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
    let handler: IActivateAudioInterfaceCompletionHandler = ProcessLoopbackActivation {
        result: activation_result.clone(),
        event: event.clone(),
    }
    .into();
    let finalized_timing;

    // SAFETY: COM is initialized; activation parameters remain valid for the async-call entry;
    // the callback owns the shared event/result even after a waiter timeout; and every acquired
    // capture packet is released before the next GetBuffer call.
    unsafe {
        let _operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &<IAudioClient as Interface>::IID,
            Some(&*activation_variant),
            &handler,
        )
        .map_err(|e| windows_error("activate system audio loopback", e))?;
        trace_windows_loopback("process-loopback activation requested");

        let result = activation_result
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut result, wait) = activation_result
            .1
            .wait_timeout_while(result, Duration::from_secs(10), |value| value.is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if result.is_none() && wait.timed_out() {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "activate system audio loopback",
                "Windows did not complete process-loopback activation within 10 seconds",
            ));
        }
        trace_windows_loopback("process-loopback activation completed");
        let audio_client = result
            .take()
            .ok_or_else(|| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "activate system audio loopback",
                    "Windows completed process-loopback activation without a result",
                )
            })??
            .0;
        trace_windows_loopback("process-loopback audio client transferred within MTA");
        let capture: IAudioCaptureClient = audio_client
            .GetService()
            .map_err(|e| windows_error("open system audio capture client", e))?;
        let output_path = std::path::Path::new(path);
        let parent = output_path.parent().ok_or_else(|| {
            RecordError::new(
                error_codes::IO,
                "reserve system audio staging",
                "system audio path has no parent",
            )
        })?;
        let (temp_path, staging) = record_recovery::create_staging_file(parent, "system-wav")
            .map_err(|error| {
                RecordError::new(
                    error_codes::IO,
                    "reserve system audio staging",
                    error.to_string(),
                )
            })?;
        let spec = hound::WavSpec {
            channels: PROCESS_LOOPBACK_CHANNELS,
            sample_rate: PROCESS_LOOPBACK_SAMPLE_RATE,
            bits_per_sample: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = match hound::WavWriter::new(staging, spec) {
            Ok(writer) => writer,
            Err(error) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(RecordError::new(
                    error_codes::IO,
                    "create system audio",
                    error.to_string(),
                ));
            }
        };
        if let Err(error) = audio_client.Start() {
            drop(writer);
            let _ = std::fs::remove_file(&temp_path);
            return Err(windows_error("start system audio loopback", error));
        }
        trace_windows_loopback("process-loopback audio client started");

        let capture_result = (|| -> Result<SystemAudioTimingTracker> {
            let mut timing = SystemAudioTimingTracker::default();
            let mut packets = 0_u64;
            let mut silent_packets = 0_u64;
            let mut saw_event = false;
            'capture: while !stop.load(Ordering::Relaxed) && capture_started.elapsed() < limit {
                let wait = WaitForSingleObject(event.0, 50);
                if wait == windows::Win32::Foundation::WAIT_FAILED {
                    return Err(windows_error(
                        "wait for system audio",
                        windows::core::Error::from_thread(),
                    ));
                }
                if !saw_event {
                    trace_windows_loopback(format_args!(
                        "process-loopback first event wait returned {wait:?}"
                    ));
                    saw_event = true;
                }
                loop {
                    if stop.load(Ordering::Relaxed) || capture_started.elapsed() >= limit {
                        break 'capture;
                    }
                    if packets == 0 {
                        trace_windows_loopback("process-loopback requesting first packet size");
                    }
                    let packet_frames = capture
                        .GetNextPacketSize()
                        .map_err(|e| windows_error("read system audio packet size", e))?;
                    if packet_frames == 0 {
                        break;
                    }
                    if packets == 0 {
                        trace_windows_loopback(format_args!(
                            "process-loopback first packet announced: {packet_frames} frames"
                        ));
                    }
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0;
                    let mut flags = 0;
                    if packets == 0 {
                        trace_windows_loopback("process-loopback acquiring first packet");
                    }
                    capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .map_err(|e| windows_error("read system audio packet", e))?;
                    // Timestamp the real packet at acquisition, before decoding or
                    // disk I/O can add scheduler delay to the timeline offset.
                    let packet_received_at = capture_started.elapsed();
                    if packets == 0 {
                        trace_windows_loopback(format_args!(
                            "process-loopback first packet acquired: {frames} frames, flags {flags:#x}"
                        ));
                    }
                    let byte_len = usize::try_from(frames)
                        .ok()
                        .and_then(|count| {
                            count.checked_mul(usize::from(PROCESS_LOOPBACK_BLOCK_ALIGN))
                        })
                        .ok_or_else(|| {
                            RecordError::new(
                                error_codes::CAPTURE,
                                "read system audio packet",
                                "WASAPI packet byte size overflowed",
                            )
                        });
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    packets += 1;
                    if silent {
                        silent_packets += 1;
                    }
                    let decoded = byte_len.and_then(|len| {
                        let packet = if silent || data.is_null() {
                            None
                        } else {
                            Some(std::slice::from_raw_parts(data, len))
                        };
                        super::super::loopback_pcm::decode_process_loopback_packet(
                            packet, frames, silent,
                        )
                    });
                    let release = capture.ReleaseBuffer(frames);
                    let decoded = decoded?;
                    release.map_err(|e| windows_error("release system audio packet", e))?;
                    if packets == 1 {
                        trace_windows_loopback("process-loopback first packet released");
                    }
                    timing.record_packet(decoded.len(), packet_received_at)?;
                    for sample in decoded {
                        writer.write_sample(sample).map_err(|e| {
                            RecordError::new(error_codes::IO, "write system audio", e.to_string())
                        })?;
                    }
                }
            }
            let elapsed = capture_started.elapsed().min(limit);
            trace_windows_loopback(format_args!(
                "process-loopback ended after {} ms: {packets} packets, {silent_packets} silent, {} samples",
                elapsed.as_millis(),
                timing.written_samples()
            ));
            Ok(timing)
        })();
        let stopped = audio_client.Stop();
        trace_windows_loopback("process-loopback stop returned");
        let timing = match capture_result {
            Ok(capture) => capture,
            Err(error) => {
                drop(writer);
                let _ = std::fs::remove_file(&temp_path);
                return Err(error);
            }
        };
        if let Err(error) = stopped {
            drop(writer);
            let _ = std::fs::remove_file(&temp_path);
            return Err(windows_error("stop system audio loopback", error));
        }
        if let Err(error) = writer.finalize() {
            let _ = std::fs::remove_file(&temp_path);
            return Err(RecordError::new(
                error_codes::IO,
                "finalize system audio",
                error.to_string(),
            ));
        }
        trace_windows_loopback("process-loopback WAV finalized");
        if let Err(error) = record_recovery::publish_new_synced(&temp_path, output_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(RecordError::new(
                error_codes::IO,
                "publish system audio",
                error.to_string(),
            ));
        }
        trace_windows_loopback(format_args!(
            "process-loopback WAV written: {} samples",
            timing.written_samples()
        ));
        // The event handle must outlive every COM interface that can still signal it.
        trace_windows_loopback("process-loopback dropping capture client");
        drop(capture);
        trace_windows_loopback("process-loopback capture client dropped");
        trace_windows_loopback("process-loopback dropping audio client");
        drop(audio_client);
        trace_windows_loopback("process-loopback audio client dropped");
        trace_windows_loopback("process-loopback dropping event");
        drop(event);
        trace_windows_loopback("process-loopback event dropped");
        finalized_timing = timing;
    }
    Ok(SystemAudioCapture {
        path: path.to_string(),
        first_packet_offset_ms: finalized_timing.first_packet_offset_ms(),
    })
}
