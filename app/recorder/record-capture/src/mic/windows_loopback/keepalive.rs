//! Best-effort silent WASAPI render keepalive and QPC packet-start helper.
//!
//! Keeps the Windows default render graph active while process-loopback runs so
//! short intermittent playback sounds do not suffer endpoint idle/close gaps.

use super::activation::ComApartment;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub(super) struct SilentRenderKeepalive {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SilentRenderKeepalive {
    pub(super) fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("cut-loopback-keepalive".into())
            .spawn(move || run_keepalive(&worker_stop))
            .ok();
        Self { stop, thread }
    }
}

impl Drop for SilentRenderKeepalive {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run_keepalive(stop: &AtomicBool) {
    let Ok(_com) = ComApartment::initialize() else {
        return;
    };
    unsafe {
        let Ok(enumerator): windows::core::Result<IMMDeviceEnumerator> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return;
        };
        let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
            return;
        };
        let Ok(client): windows::core::Result<IAudioClient> = device.Activate(CLSCTX_ALL, None)
        else {
            return;
        };
        let Ok(mix_fmt) = client.GetMixFormat() else {
            return;
        };
        let init = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            0,
            1_000_000, // 100 ms buffer
            0,
            mix_fmt,
            None,
        );
        CoTaskMemFree(Some(mix_fmt.cast()));
        if init.is_err() {
            return;
        }
        let Ok(buffer_frames) = client.GetBufferSize() else {
            return;
        };
        let Ok(render): windows::core::Result<IAudioRenderClient> = client.GetService() else {
            return;
        };
        if let Ok(_ptr) = render.GetBuffer(buffer_frames) {
            let _ = render.ReleaseBuffer(buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
        }
        if client.Start().is_err() {
            return;
        }
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(35));
            if stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(padding) = client.GetCurrentPadding() else {
                break;
            };
            let avail = buffer_frames.saturating_sub(padding);
            if avail > 0 && render.GetBuffer(avail).is_ok() {
                let _ = render.ReleaseBuffer(avail, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
            }
        }
        let _ = client.Stop();
    }
}

pub(super) fn packet_byte_len(frames: u32) -> record_core::Result<usize> {
    usize::try_from(frames)
        .ok()
        .and_then(|count| {
            count.checked_mul(usize::from(super::activation::PROCESS_LOOPBACK_BLOCK_ALIGN))
        })
        .ok_or_else(|| {
            record_core::RecordError::new(
                record_core::error_codes::CAPTURE,
                "read system audio packet",
                "WASAPI packet byte size overflowed",
            )
        })
}

pub(super) fn open_staging_wav(
    path: &str,
) -> record_core::Result<(
    std::path::PathBuf,
    hound::WavWriter<std::io::BufWriter<std::fs::File>>,
)> {
    let parent = std::path::Path::new(path).parent().ok_or_else(|| {
        record_core::RecordError::new(
            record_core::error_codes::IO,
            "reserve system audio staging",
            "system audio path has no parent",
        )
    })?;
    let (temp_path, staging) =
        record_recovery::create_staging_file(parent, "system-wav").map_err(|e| {
            record_core::RecordError::new(
                record_core::error_codes::IO,
                "reserve system audio staging",
                e.to_string(),
            )
        })?;
    let spec = hound::WavSpec {
        channels: super::activation::PROCESS_LOOPBACK_CHANNELS,
        sample_rate: super::activation::PROCESS_LOOPBACK_SAMPLE_RATE,
        bits_per_sample: super::activation::PROCESS_LOOPBACK_BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = hound::WavWriter::new(std::io::BufWriter::new(staging), spec).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        record_core::RecordError::new(
            record_core::error_codes::IO,
            "create system audio",
            e.to_string(),
        )
    })?;
    Ok((temp_path, writer))
}

pub(super) fn query_qpc_freq() -> i64 {
    let mut freq = 0_i64;
    if unsafe { QueryPerformanceFrequency(&mut freq) }.is_ok() {
        freq
    } else {
        0
    }
}

pub(super) fn packet_start_elapsed(
    qpc_freq: i64,
    qpc_pos_100ns: u64,
    received_at: Duration,
) -> Option<Duration> {
    if qpc_freq <= 0 || qpc_pos_100ns == 0 {
        return None;
    }
    let mut qpc_now = 0_i64;
    unsafe { QueryPerformanceCounter(&mut qpc_now) }.ok()?;
    let now_100ns =
        u64::try_from((i128::from(qpc_now) * 10_000_000) / i128::from(qpc_freq)).ok()?;
    let age_100ns = now_100ns.checked_sub(qpc_pos_100ns)?;
    Some(received_at.saturating_sub(Duration::from_nanos(age_100ns.saturating_mul(100))))
}

pub(super) struct LoopbackPacketMeta {
    pub(super) qpc_freq: i64,
    pub(super) qpc_pos_100ns: u64,
    pub(super) silent: bool,
    pub(super) discontinuity: bool,
    pub(super) received_at: Duration,
}

pub(super) fn write_timed_packet<W: std::io::Write + std::io::Seek>(
    writer: &mut hound::WavWriter<W>,
    timing: &mut crate::system_audio_timing::SystemAudioTimingTracker,
    level: Option<&super::super::RollingAudioLevel>,
    decoded: &[i16],
    meta: LoopbackPacketMeta,
) -> record_core::Result<()> {
    let packet_start = packet_start_elapsed(meta.qpc_freq, meta.qpc_pos_100ns, meta.received_at);
    let gap_samples = timing.record_timed_packet(crate::system_audio_timing::TimedAudioPacket {
        samples: decoded.len(),
        qpc_position_100ns: meta.qpc_pos_100ns,
        discontinuity: meta.discontinuity,
        is_silent: meta.silent || decoded.iter().all(|&sample| sample == 0),
        elapsed_at_capture: meta.received_at,
        packet_start_elapsed: packet_start,
        sample_rate: super::activation::PROCESS_LOOPBACK_SAMPLE_RATE,
        channels: super::activation::PROCESS_LOOPBACK_CHANNELS,
    })?;
    if let Some(level) = level {
        level.observe_i16(decoded);
    }
    let write_err = |e: hound::Error| {
        record_core::RecordError::new(
            record_core::error_codes::IO,
            "write system audio",
            e.to_string(),
        )
    };
    for _ in 0..gap_samples {
        writer.write_sample(0_i16).map_err(write_err)?;
    }
    for &sample in decoded {
        writer.write_sample(sample).map_err(write_err)?;
    }
    Ok(())
}
